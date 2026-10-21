//! The four controls a gradient is dialled with: the [`GradientPreview`] the
//! group stands under, the [`SpectrumBar`] that sets its hue arc, and the two
//! [`SpreadBar`]s that set its brightness and chroma pairs.
//!
//! One module because they are one control between them. They write to a single
//! [`Gradient`], they reset to the single home their caller hands each of them,
//! and the preview is the only place what the other three compose can be seen at
//! all.

use egui::{Color32, CornerRadius, Response, Sense, TextStyle, Ui, Vec2};
use harmonigraph_scene::{gradient_color, hue_circle, Gradient, HUE_CIRCLE_N, PITCH_LUT_N};

use super::bar::{
    bar_radius, bar_width, drag, elided_name, grip_radius, paint_thumbs, poised, track_fill,
    BAR_LABEL_GAP, BAR_TEXT_PAD, GRAB_PX, HANDLE_INSET, HANDLE_REACH_SHARE, HANDLE_W, TEXT_GAP,
};
use super::mesh::gradient_strip;
use super::text_at;
use crate::panes::scene_color;
use crate::theme;

/// Segments the spectrum bar's track is drawn in. Twice [`HUE_CIRCLE_N`], so
/// every sample of the hue circle lands on a segment BOUNDARY where it is
/// drawn exactly, and only the vertices between two samples are interpolated.
const SPECTRUM_SEGMENTS: usize = HUE_CIRCLE_N * 2;

/// One turn of the hue circle, in degrees, and so the whole width of a
/// [`SpectrumBar`]'s track.
///
/// The bar can stand for the span knob's entire travel only because
/// [`Gradient::MAX_HUE_SPAN`] is exactly this. Widen that constant and
/// `sanitized` would accept spans the bar cannot draw: the handle would park at
/// the right edge and stop answering while the value went on growing, with
/// nothing to fail at compile time.
/// `the_spectrum_bar_track_is_a_whole_turn_of_the_span_knob` is what catches it.
const FULL_TURN: f32 = 360.0;

/// How far back the hues a gradient does not reach are held. Through alpha over
/// the well rather than a blend toward a fixed grey, so the bar sits on
/// whatever the pane is instead of ringing itself in a slightly wrong color.
const UNCLAIMED_ALPHA: f32 = 74.0 / 255.0;

/// The `L*` and the chroma fraction the track's hue circle is drawn at —
/// FIXED, and not the gradient's own pair.
///
/// The track is the HUE control, and the two knobs it does not set have bars of
/// their own directly beneath it. A circle drawn at the gradient's own
/// brightness and chroma answers those bars as well, and answers them by going
/// blind: at a chroma of 0 the whole turn is grey, at a dark setting it is
/// nearly black, and both are one drag away on the bars beneath. The one control that has to show
/// hues would show none at the settings a reader is most likely to be dialling
/// their way out of. What the six knobs COMPOSE is the [`GradientPreview`]
/// the group stands under, which is drawn from the same table the picture is.
///
/// Mid `L*`, so no part of the circle is crowded against either end of the
/// axis, and short of the gamut boundary for the reason [`Gradient`]'s own
/// default chroma is short of it: at a fraction of 1 the maximum's kinks
/// between the sRGB primaries show up as bumps in a sweep whose whole job is to
/// read as an even turn.
const TRACK_LIGHTNESS: f32 = 60.0;
/// The chroma fraction that goes with [`TRACK_LIGHTNESS`]. High, because the
/// fraction is of the floor every hue can hold rather than of each hue's own
/// ceiling, and a track drawn low on that axis reads as washed out.
const TRACK_CHROMA: f32 = 0.85;

/// The colour at `hue` degrees on `circle`, a [`hue_circle`], interpolated
/// between its two nearest samples.
fn circle_at(circle: &[glam::Vec4; HUE_CIRCLE_N], hue: f32) -> glam::Vec4 {
    let f = hue.rem_euclid(FULL_TURN) / FULL_TURN * HUE_CIRCLE_N as f32;
    let i0 = f.floor() as usize % HUE_CIRCLE_N;
    circle[i0].lerp(circle[(i0 + 1) % HUE_CIRCLE_N], f - f.floor())
}

/// Height of a [`GradientPreview`]. Shorter than a row, because it is a
/// picture and not a control: nothing on it can be dragged, and a band standing
/// as tall as the bars under it would read as a fourth bar that has lost its
/// handle.
const PREVIEW_H: f32 = 14.0;

/// [`PREVIEW_H`] at this scale.
///
/// Shared with the settings tests, whose sweeps find a bar by its own height
/// and a preview by this one. A test restating the number instead would keep
/// passing on the day the preview changed height, by then measuring nothing.
pub(crate) fn preview_height(scale: f32) -> f32 {
    PREVIEW_H * scale
}

/// The pane showing between a [`SpectrumBar`]'s flip button and the track
/// beside it. Narrow enough to be tighter than the pane's own row spacing, so
/// the two read as one control rather than as a button with a bar near it.
const FLIP_GAP: f32 = 2.0;

/// What a [`SpectrumBar`]'s track measures in a column `column` points wide:
/// the column, less the flip button at the right end and the gap beside it.
///
/// Shared with the settings tests, whose sweep pins every bar in a pane to the
/// width of its column. This is the one bar that is narrower, and the sweep has
/// to know by how much or it is choosing between failing on a bar that is
/// correct and passing on one that has stopped tracking the column.
pub(crate) fn spectrum_track_width(column: f32, scale: f32) -> f32 {
    (column - (FLIP_W + FLIP_GAP) * scale).max(0.0)
}

/// Width of the flip button at the RIGHT end of a [`SpectrumBar`], taken out of
/// the row the bar already has rather than off a row of its own.
///
/// It costs the track that much travel, which is the whole trade and a cheap
/// one: the track stands for a whole turn at any length, so a shorter one is a
/// coarser drag and nothing else — 18pt of 400 is a twentieth of a degree per
/// pixel. A row costs 22pt of a column that already scrolls.
///
/// The far end from where the arc starts, so the button sits past the end of
/// the reading rather than in front of it: the track is cut at the arc's own
/// start and fills from the left, and the thing that reverses it belongs after
/// what it reverses. It costs the button nothing to sit there: the settings
/// pane's scroll bar runs in the dock's own gutter, outside the content box,
/// so the button ends where the bar's lane begins rather than under it —
/// `nothing_is_drawn_under_a_settings_pane_scroll_bar` holds that lane empty.
const FLIP_W: f32 = 18.0;

/// [`FLIP_W`] at this scale, for the settings sweep that holds the lane the
/// button ends against empty — and has to know it drew the button to say so.
#[cfg(test)]
pub(crate) fn flip_button_width(scale: f32) -> f32 {
    FLIP_W * scale
}

/// The name a [`SpectrumBar`] writes along its own track.
///
/// Here rather than handed in by the caller, for the reason [`Spread::label`]
/// is: a bar over a gradient is a bar over the hue pair whichever gradient it
/// is, and a name passed separately is a way for one pane to call it something
/// the next pane does not.
const SPAN_LABEL: &str = "Hue";

/// The color a [`SpectrumBar`] writes its own name in: the ground the bar sits
/// on, which is the one text run in the dock not drawn in the theme's text.
///
/// **The circle is drawn at ONE lightness ([`TRACK_LIGHTNESS`]), so a word on it
/// has exactly two possible grounds** whatever the six knobs say: the circle at
/// `L*` 60 where the arc claims it, and that same circle held back to the well
/// beyond the handle. Nothing reads well on both — the theme's text is about
/// 2.5:1 against the claimed half and 6:1 against the held-back one, and the
/// well is those two the other way round — so the name takes the color that
/// reads where the name actually stands. It is pinned to the track's LEFT end,
/// which is the CLAIMED end at every arc wide enough to reach past it, about 60
/// degrees on the column this pane opens at.
///
/// What that costs is a grey ramp and the arcs narrower than the name is wide: there
/// the word stands on held-back color and goes quiet. That is the right way for
/// it to fail — those are the settings where the bar has least to say, and the
/// alternative is the whole rest of the range spent at 2.5:1.
///
/// The theme's own well rather than a black named here, so a re-skin moves the
/// name with the ground it is drawn to match.
fn span_name_color() -> Color32 {
    theme::well()
}

/// Which part of a [`SpectrumBar`]'s track a drag took hold of. Decided once,
/// at drag-start, and remembered for the gesture, exactly as [`Grab`] is: a
/// span dragged to nothing would otherwise hand the rest of the gesture to the
/// rotate branch the moment the handle reached the left edge.
///
/// [`Grab`]: super::range::Grab
#[derive(Clone, Copy, Default)]
enum SpectrumGrab {
    /// The far end of the arc — how far round the circle the gradient walks.
    #[default]
    Span,
    /// The circle itself, sliding under a fixed left edge. `held` is the hue
    /// that was under the pointer when the gesture started, and the whole
    /// gesture is "keep that hue under the pointer"; fixed for the gesture, so
    /// a turn never reads back the circle it is itself moving.
    Rotate { held: f32 },
    /// A press that landed off the track, which for this widget means on the
    /// [`GradientPreview`] above it — or in the pane's own row spacing either
    /// side.
    ///
    /// A rectangle the preview is not inside is NOT enough to keep a press on
    /// it out of the track, and that is the whole reason this variant exists:
    /// egui's hit test gathers every widget within
    /// `interaction.interact_radius` of the pointer and, when the press hits
    /// none of them squarely, gives it to the nearest. At the default radius of
    /// 5 the track reaches five points past its own edges — further than the
    /// 4pt of row spacing above it, and the preview senses hover alone, so a
    /// press on the preview's lower edge is a press the widget is handed and
    /// has to decline by position.
    ///
    /// Remembered for the gesture like the other two, so a drag that started
    /// off the track does not catch hold the moment the pointer crosses onto
    /// it.
    Outside,
}

/// The gradient itself, end to end at a fixed scale: the picture the bars under
/// it compose, low note (or silence) on the left.
///
/// **The one place a reader sees all six knobs at once**, which is why it
/// stands above the three bars rather than beside any one of them. Each bar
/// below is a picture of the two numbers IT sets — the hue arc on a
/// [`SpectrumBar`], the brightness and chroma pairs on a [`SpreadBar`] each —
/// and none of them can show what the six make together. It is also the only
/// one of the four that survives every setting: at a span of zero the arc has
/// no width at all, and a single hue over a brightness ramp is a real gradient
/// that the hue track alone would draw as one column of nothing.
///
/// Read out of the same table the lattice draws from, so the preview cannot
/// drift from the picture. A band mixed here from the widget's own idea of the
/// gradient would be a second definition of the color, wrong the first time
/// either changed.
///
/// A picture and not a control: it senses hover, so it can carry a tooltip, and
/// there is nothing on it to drag. What keeps a press ON it from reaching the
/// bar below is that bar's own position check — see [`SpectrumGrab::Outside`],
/// where the reason a rectangle is not enough is written out.
///
/// Full column width and no well beneath it, unlike every bar: it is opaque end
/// to end and covers whatever ground it is given, and a recessed track under a
/// picture would say there is a value in there somewhere.
///
/// **Its space is taken first and its paint happens last, and the two are
/// separate calls for exactly that reason.** A settings pane draws top-down, so
/// a picture drawn where it STANDS is drawn from the value the bars below it
/// were handed rather than the one they just wrote: every frame of every drag
/// in the group would show the gradient as it was one frame ago, above three
/// bars showing it as it is. `harmonigraph_scene::color`'s `LUT_SLOTS` counts
/// that frame from the other side — everything above a bar reads what the bar
/// wrote last frame, which is why the bar re-reads before painting — and a
/// picture is the one thing in a settings pane that can afford neither the lag
/// nor a row further down. `a_spectrum_drag_draws_the_preview_it_just_set` is
/// what holds the order.
///
/// Reserving is what makes that possible: the row is claimed at the top of the
/// group, so the bars land under it, and the colors are read at the bottom of
/// the group, after every one of them has written.
pub struct GradientPreview {
    rect: egui::Rect,
    id: egui::Id,
}

impl GradientPreview {
    /// Claim the row, before the bars that write the gradient are drawn.
    pub fn reserve(ui: &mut Ui) -> Self {
        let scale = theme::ui_scale(ui.ctx());
        let width = bar_width(ui);
        let (id, rect) = ui.allocate_space(Vec2::new(width, preview_height(scale)));
        GradientPreview { rect, id }
    }

    /// Paint it, after them.
    pub fn show(self, ui: &Ui, gradient: &Gradient) -> Response {
        let scale = theme::ui_scale(ui.ctx());
        let response = ui.interact(self.rect, self.id, Sense::hover());
        // Off screen nothing below is needed; see the same line in `ValueBar`.
        if !ui.is_rect_visible(self.rect) {
            return response;
        }
        // Read the way every picture reads the table, through `gradient_color`,
        // since a bent gradient's entries are not evenly spaced along the bar
        // (`LutSpacing`). A column per point rather than per entry, so the
        // entries a steep bend packs into a few points of the bar are each
        // still drawn; never fewer columns than entries.
        let gradient = gradient.sanitized();
        // Rounded at both ends: the preview is a band in its own right, and
        // each of its ends meets the pane rather than another shape.
        let corner = f32::from(bar_radius(scale));
        let columns = (self.rect.width().round() as usize).max(PITCH_LUT_N - 1);
        gradient_strip(ui.painter(), self.rect, columns, (corner, corner), |p| {
            scene_color(gradient_color(p, gradient), 1.0)
        });
        response
    }
}

/// The pitch gradient's hue arc, as two pieces of one control: a track carrying
/// a full turn of the color circle, CUT at the arc's own start, with the stretch
/// the gradient walks lit from the track's left edge and the hues it does not
/// reach dimmed beyond it — and past the right end of that, the button which
/// reverses it. The bar's name stands on the circle at the left.
///
/// What the arc COMPOSES with the other four knobs is not here at all: it is
/// the [`GradientPreview`] the group stands under. This bar is the hue pair
/// and nothing else, and the two spread bars below it are the other four.
///
/// Every piece wears the shared [`CONTROL_RADIUS`](theme::CONTROL_RADIUS) and
/// sits on the pane, with no frame drawn round the pair: the button is a button
/// down to the table its colors are read out of, and the circle rounds exactly
/// like the fill of a [`ValueBar`] above it. A well large enough to hold both
/// would ring the control in a border nothing else in a settings pane wears —
/// see [`gradient_strip`], which is where the alternative was paid for.
///
/// Drag the handle to set how far round the circle the range walks; drag the
/// track to turn the whole circle under it; double-click to reset. Which
/// DIRECTION it runs is the flip button, not a gesture — see below.
///
/// **Cut at the start, which is what makes a circle fit on a bar.** Hue wraps,
/// so an arc laid on a fixed 0..360 track is drawn in two pieces whenever it
/// crosses the seam — and the default arc does, running 260 through 0 to 90,
/// which would put its two halves at opposite ends of the bar with the colors
/// it never uses in between. Pinning the START to the left edge instead means
/// the arc is one piece at every setting and always reads low-to-high, left to
/// right; what moves is the circle behind it. The cost is that the bar cannot
/// say where on the circle it is in absolute terms, which is a number nobody
/// reads a color off anyway — the track is painted in the colors themselves.
///
/// **The circle is hue and nothing else.** It is [`hue_circle`] at one fixed
/// lightness and chroma ([`TRACK_LIGHTNESS`]) — read out of a table rather than
/// mixed here, so it cannot drift from what the lattice draws — at full
/// strength over the stretch the arc claims and dimmed beyond it. Brightness
/// and chroma show up in the preview above instead, which is where they can be
/// read against the bars that set them.
///
/// One circle either side of the handle is also what makes the two halves of
/// the track meet FLUSH at every setting, rather than only where both ramps
/// happen to be flat: the same hue is drawn at two strengths, so where the arc
/// stops is a change of strength and nothing else.
///
/// **The name stands ON the circle, in the place every other bar puts its own,
/// and is the one text run in the dock drawn dark rather than light** — see
/// [`span_name_color`], where the two grounds a word on this track can have are
/// counted and the choice between them made. The circle runs the whole track
/// rather than starting past a gutter, which is what keeps the arc reading from
/// the bar's own left edge.
///
/// The readout is held to the stretch RIGHT of the name, exactly as a
/// [`RangeBar`]'s numbers are, so the two text runs cannot meet however wide
/// the arc grows. The handle is not: it travels the whole circle, name
/// included, being the part you operate and drawn over everything, and both
/// runs are knocked out through it ([`paint_thumbs`]) as on every other bar.
///
/// [`paint_thumbs`]: super::bar::paint_thumbs
///
/// **The flip is a button because the track cannot carry the gesture.** The arc
/// is laid out from its own start, so both directions draw the same stretch of
/// color in the same place and there is nothing on the track to drag the other
/// way. It lives in the bar's own row rather than above or below it because a
/// settings column is short of rows and not of width: the two things a reader
/// wants together are the arc and the direction it runs, and a row spent on one
/// button pushes every knob under it further down a pane that already scrolls.
/// It sits past the far end of the arc — see [`FLIP_W`] for why that end.
///
/// **What the flip changes on screen is the sign, and the HUE of the circle and
/// the preview alike.** Each reads low note at the left — the circle from the
/// hue the bottom of the range takes, the preview from the color it takes — so
/// the arc runs the other way round the circle in both, which is exactly the
/// change drawn where the change is. Neither RAMP turns around with it:
/// [`Gradient::flipped`] rewrites the hue pair and carries brightness and
/// chroma through untouched, so the preview keeps its dark end where it was and
/// the two spread bars read out the same pair after a flip as before it. The
/// readout spells the direction out on top of all that, because an arc and its
/// flip claim exactly the same colors.
///
/// [`ValueBar`]: super::value::ValueBar
/// [`RangeBar`]: super::range::RangeBar
pub struct SpectrumBar<'a> {
    gradient: &'a mut Gradient,
    home: Gradient,
}

impl<'a> SpectrumBar<'a> {
    /// A bar over `gradient`, which a double-click on the track takes the ARC
    /// home to `home`'s — only its two hue fields, those being the only ones
    /// the track sets.
    ///
    /// `home` is the gradient the picture this bar dials opens on, and every
    /// caller names it because no default here could be right for all of them.
    /// Not `Gradient::default()`, which is the type's own CIELAB-converted arc:
    /// `ViewConfig::default` COMPOSES its gradient instead, and a reset landing
    /// on the type's default puts the bar on a pair the plugin has never opened
    /// on — with no text entry to dial the shipped one back. Not the lattice's
    /// either, since the Spectral pane's heatmap has a default of its own and a
    /// reset there landing on the lattice's arc is the same unrecoverable jump
    /// one pane over. Read it off the fresh config rather than restating it,
    /// so a reset follows any later change to the shipped defaults.
    pub fn new(gradient: &'a mut Gradient, home: Gradient) -> Self {
        SpectrumBar { gradient, home }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let scale = theme::ui_scale(ui.ctx());
        let width = bar_width(ui);
        // Space first, senses after: the row holds two controls side by side,
        // and each is interacted with over its OWN rectangle. That is what
        // keeps a press on the flip button out of the track — a button that
        // senses clicks and not drags produces no drag hit at all, so there is
        // nothing for the track to inherit at any distance.
        //
        // One rectangle sensed for both would put that on a position check
        // instead: the widget would take the drag and have to decline it by
        // asking `drag` where the press landed, exactly as the preview
        // above IS declined. Two rectangles rather than one check because a
        // check can only be reached once egui calls the press a drag, and the
        // frames before that are ones the button spends looking pressed while
        // the track quietly holds the gesture.
        let flip_gap = FLIP_GAP * scale;
        let (id, rect) = ui.allocate_space(Vec2::new(width, theme::row_height(scale)));
        // The track's width is what the circle and the settings sweep both
        // measure, so the row is laid out from it rather than from the button:
        // a column too narrow to leave the track anything then gives the button
        // the row, which is the right way round. A coarse handle beats an
        // unreachable one, but a button with no width cannot be pressed at all.
        let split = rect.left() + spectrum_track_width(rect.width(), scale);
        let track_rect = egui::Rect::from_min_max(rect.min, egui::pos2(split, rect.bottom()));
        let flip_rect = egui::Rect::from_min_max(
            egui::pos2((split + flip_gap).min(rect.right()), rect.top()),
            rect.max,
        );
        let mut response = ui.interact(track_rect, id.with("track"), Sense::click_and_drag());
        let flip = ui.interact(flip_rect, id.with("flip"), Sense::click()).on_hover_text(
            "Run the spectrum the other way round the circle — the same \
                 colors, low and high swapped",
        );

        // ---- The name, and the stretch it keeps the readout out of ----------
        // Laid out HERE rather than with the rest of the paint, so its width is
        // in hand before anything else is placed against it.
        let painter = ui.painter();
        // Whether a point is on the control, as opposed to on the picture above
        // it or in the row spacing either side. The track's sensed rectangle
        // stops at its own edges, and this is still the only thing standing
        // between a press off it and a gesture — see [`SpectrumGrab::Outside`]
        // for why the rectangle is not enough.
        let on_track = |p: &egui::Pos2| track_rect.contains(*p);
        // The readout in full text at every state, as every text is; the name is
        // one colour too, drawn to be read against the hue behind it
        // ([`span_name_color`]).
        let text_color = theme::text();
        let mono = TextStyle::Monospace.resolve(ui.style());
        let text_gap = TEXT_GAP * scale;
        // Room kept clear for the readout, measured from the widest string it
        // can produce rather than from the span in the bar now — a RangeBar
        // reserves its ends the same way and for the same reason: a name that
        // re-elides as the number gains a digit wobbles under the pointer
        // mid-drag. A whole turn is as long as it gets, and the sign is always
        // written.
        let reserve = painter
            .layout_no_wrap(format!("{:+.0}°", -FULL_TURN), mono.clone(), theme::text())
            .size()
            .x;
        let job = egui::text::LayoutJob::simple_singleline(
            SPAN_LABEL.to_owned(),
            TextStyle::Body.resolve(ui.style()),
            span_name_color(),
        );
        let text_pad = BAR_TEXT_PAD * scale;
        let label = elided_name(painter, job, track_rect.width(), scale, reserve);
        // Where the name ends, and so the only part of the track the readout is
        // allowed into. The name was laid out against a width with the widest
        // readout already subtracted, so what is left here holds it: the name
        // can no more be pushed off by the number than the number can push into
        // the name.
        let readable_left = track_rect.left() + text_pad + label.size().x + BAR_LABEL_GAP * scale;
        // What the handle travels: the track, less half a handle at each end,
        // so both limits — a span of zero and a whole turn — are places it can
        // stand rather than edges it merges into. Same reason as HANDLE_INSET.
        // The whole track, name and all: a handle standing in the name is the
        // same bargain a RangeBar's ends make, and the handle is the part you
        // operate.
        let travel = track_rect.shrink2(Vec2::new(HANDLE_INSET * scale, 0.0));
        // Where a gradient puts itself on that: which way round the circle it
        // runs, how much of the turn it claims, and where that leaves the
        // handle. A function rather than three bindings because the answer is
        // wanted TWICE — once for the gradient a gesture is aimed at, and again
        // for the one that gesture just wrote.
        let laid_out = |g: Gradient| {
            // A span of zero has no direction of its own, and opening rightward
            // is the useful reading: dragging the handle out of nothing then
            // grows an arc rather than needing the sign set first. `sanitized`
            // is what makes the test sound, by keeping -0.0 out of the field.
            let winding = if g.hue_span < 0.0 { -1.0f32 } else { 1.0 };
            let claimed = (g.hue_span / FULL_TURN).abs().clamp(0.0, 1.0);
            (winding, claimed, travel.left() + travel.width() * claimed)
        };

        // ---- Interaction ----------------------------------------------------
        // Ahead of the snapshot below, so the frame that flips is the frame
        // that draws it flipped — the same reason the paint re-reads the
        // gradient rather than the value a drag was aimed at.
        if flip.clicked() {
            // The arithmetic lives on the gradient rather than here: what a
            // flip IS — the far end becoming the near one, so the arc keeps its
            // place on the circle — is a property of the gradient that this bar
            // previews and a test pins, and a second spelling of it here is the
            // one that would drift.
            *self.gradient = self.gradient.flipped();
            response.mark_changed();
        }
        let aimed = self.gradient.sanitized();
        let (winding, _, handle_x) = laid_out(aimed);
        // Where a point across the circle sits on it, as a signed offset in
        // degrees from the hue at the left edge.
        let offset_at = |x: f32| {
            ((x - travel.left()) / travel.width().max(1.0)).clamp(0.0, 1.0) * FULL_TURN * winding
        };
        // What a press landing at `origin` takes hold of, with the handle
        // standing at `handle_x`: the press's own rule, and the one the handle
        // is lit by under a pointer that has not pressed yet.
        let grab_at = |origin: egui::Pos2, handle_x: f32| {
            if !on_track(&origin) {
                SpectrumGrab::Outside
            } else if (origin.x - handle_x).abs() <= GRAB_PX {
                SpectrumGrab::Span
            } else {
                SpectrumGrab::Rotate { held: aimed.hue_start + offset_at(origin.x) }
            }
        };
        let clicked_track = response.interact_pointer_pos().is_some_and(|p| on_track(&p));
        if response.double_clicked() && clicked_track {
            let home = self.home.sanitized();
            self.gradient.hue_start = home.hue_start;
            self.gradient.hue_span = home.hue_span;
            response.mark_changed();
        }
        // All three of the grab's questions are asked of where the press
        // LANDED — see `drag` — and none of them of where the pointer has since
        // got to. Whether the gesture is ours, because a press that began on
        // the preview is already over the track by the first live frame; handle
        // or track, because a press inside the handle's reach is already clear
        // of it; and the hue a turn holds, because the gesture begins where the
        // hand put it down and turning the circle by less than the pointer has
        // travelled is a gesture that starts behind and stays there.
        let held = drag(ui, &response, |aim| grab_at(aim, handle_x));
        if let Some((grab, p)) = held {
            let next = match grab {
                // The magnitude only. Its SIGN is the flip button's, and
                // leaving it there is what lets the handle reach zero without
                // the arc turning inside out on the way past.
                SpectrumGrab::Span => {
                    Some(Gradient { hue_span: winding * offset_at(p.x).abs(), ..aimed })
                }
                SpectrumGrab::Rotate { held } => Some(Gradient {
                    hue_start: harmonigraph_scene::wrap_degrees(held - offset_at(p.x)),
                    ..aimed
                }),
                SpectrumGrab::Outside => None,
            };
            if let Some(next) = next.filter(|next| *next != *self.gradient) {
                *self.gradient = next;
                response.mark_changed();
            }
        }
        let holding = held.map(|(grab, _)| grab);

        // Off screen nothing below is needed; see the same line in `ValueBar`.
        if !ui.is_rect_visible(rect) {
            return response;
        }
        // ---- Paint ----------------------------------------------------------
        // The gradient read BACK, not the snapshot the gesture was aimed at. A
        // drag has just written it, and painting the value from before that
        // write leaves the handle, the arc and the readout a whole frame behind
        // the pointer for the length of the gesture — a step of one value and
        // about 17px at a brisk drag. ValueBar and RangeBar both re-read their
        // values here for the same reason.
        let g = self.gradient.sanitized();
        let (winding, claimed, handle_x) = laid_out(g);
        let circle = hue_circle(TRACK_LIGHTNESS, TRACK_CHROMA);
        let corner = bar_radius(scale);
        let radius = CornerRadius::same(corner);
        // A well under the whole track, because the dimmed hues are drawn with
        // alpha and need a recessed ground to sit on — the same ground the
        // unfilled end of a ValueBar shows, and the color the name is written
        // in ([`span_name_color`]).
        painter.rect_filled(track_rect, radius, theme::well());
        // ONE circle across the whole track, at full strength over the stretch
        // the arc claims and held back to ground beyond it. The same hue on
        // both sides of the handle, so the two meet flush at every setting and
        // what the handle marks is how far round the turn the gradient reaches
        // — which is the only thing this track is for.
        gradient_strip(
            painter,
            track_rect,
            SPECTRUM_SEGMENTS,
            (corner as f32, corner as f32),
            |p| {
                let alpha = if claimed > 0.0 && p <= claimed { 1.0 } else { UNCLAIMED_ALPHA };
                scene_color(circle_at(&circle, g.hue_start + p * FULL_TURN * winding), alpha)
            },
        );
        let centered = |galley: &egui::Galley, x: f32| text_at(track_rect, galley, x);
        let label_pos = centered(&label, track_rect.left() + text_pad);
        painter.galley(label_pos, label.clone(), span_name_color());

        // How far round the circle the arc reaches, read out beside the handle
        // — on the dimmed side, where it sits on flat color, and on the claimed
        // side when the arc has grown too wide to leave room there, which is
        // the same bargain a [`RangeBar`]'s ends make. One number and one
        // handle, so it needs none of the arithmetic that keeps a range's TWO
        // roaming numbers out of each other. The sign is the direction, and it
        // is spelled out because the track cannot show it: an arc and its flip
        // claim exactly the same colors.
        //
        // Held clear of the NAME at its left, which is the one text run it
        // cannot slide under: the name is a word and the two would read as one
        // string. Its room came out of the name's own width above.
        let galley = painter.layout_no_wrap(format!("{:+.0}°", g.hue_span), mono, text_color);
        let reach = HANDLE_W * 0.5 * scale + text_gap;
        let outside = handle_x + reach;
        let left = if outside + galley.size().x <= track_rect.right() - text_gap {
            outside
        } else {
            handle_x - reach - galley.size().x
        };
        let left = left.clamp(
            readable_left,
            (track_rect.right() - text_gap - galley.size().x).max(readable_left),
        );
        let at = centered(&galley, left);
        painter.galley(at, galley.clone(), text_color);

        // The handle on top of everything, readout included: it is the part
        // you operate. Both runs are knocked out through it, as on every other
        // bar, so a letter of the name or a digit it stands in inverts rather
        // than disappearing behind it — and a short arc stands the handle in
        // the name.
        //
        // Lit by what is in hand (see [`grip_color`]), and on a bar with one
        // handle that still says something: unlit is a press that would TURN
        // the circle under the handle rather than take it.
        let in_hand = holding.or_else(|| poised(ui, &response).map(|p| grab_at(p, handle_x)));
        let turning = matches!(in_hand, Some(SpectrumGrab::Rotate { .. }));
        paint_thumbs(
            painter,
            track_rect,
            scale,
            grip_radius(scale),
            [(handle_x, !turning)],
            &[(label_pos, label), (at, galley)],
        );

        // ---- The flip button ------------------------------------------------
        // Painted out of the theme's own widget visuals, state for state, and
        // not out of a set of colors chosen to look like them: the fill, the
        // edge and the corner are read from the same table egui hands a
        // `Button`, and the state is picked the way `Style::interact` picks it.
        // Naming the colors here instead is how it drifts — a resting fill
        // copied correctly and a hovered edge given a scaled width the theme
        // does not scale, and a pressed state simply forgotten, so the one
        // control in the pane that does not answer a click is this one.
        let visuals = if flip.is_pointer_button_down_on() {
            &ui.style().visuals.widgets.active
        } else if flip.hovered() {
            &ui.style().visuals.widgets.hovered
        } else {
            &ui.style().visuals.widgets.inactive
        };
        painter.rect_filled(flip_rect, visuals.corner_radius, visuals.weak_bg_fill);
        if visuals.bg_stroke.width > 0.0 {
            painter.rect_stroke(
                flip_rect,
                visuals.corner_radius,
                visuals.bg_stroke,
                egui::StrokeKind::Inside,
            );
        }
        // The MARK is what this widget invents; its color is the theme's, the
        // one a button's own label is drawn in.
        flip_mark(painter, flip_rect, visuals.fg_stroke.color, scale);

        // The cursor says which gesture a press would start before committing
        // to a drag, as a RangeBar's does: the handle resizes the arc, the
        // track turns the circle under it. Off the track it says neither,
        // because a press there starts nothing — see [`SpectrumGrab::Outside`].
        // Read off the same grab the handle is lit by.
        match in_hand {
            Some(SpectrumGrab::Span) => {
                response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
            }
            Some(SpectrumGrab::Rotate { .. }) => response.on_hover_cursor(egui::CursorIcon::Grab),
            Some(SpectrumGrab::Outside) | None => response,
        }
    }
}

/// The reverse mark on a [`SpectrumBar`]'s flip button: two arrows, one over
/// the other, pointing opposite ways.
///
/// Symmetric on purpose. A flip is its own undo and the track claims the same
/// colors either way round, so an arrow committing to a direction would be
/// pointing at nothing on screen; which way the arc currently runs is the sign
/// on the readout. Painted rather than set as a glyph, because the two product
/// faces are text faces and an arrow found in whichever fallback egui reaches
/// for would be the one piece of this UI drawn in an unknown font.
fn flip_mark(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32, scale: f32) {
    let head = 3.5 * scale; // length of an arrowhead
    let half = 2.5 * scale; // and half its width
    let gap = 3.0 * scale; // how far each arrow sits off the center line
    let shaft = (rect.width() - 4.0 * scale).max(head);
    let stroke = egui::Stroke::new((1.0 * scale).max(1.0), color);
    let (left, right) = (rect.center().x - shaft * 0.5, rect.center().x + shaft * 0.5);
    for (y, tip, tail) in
        [(rect.center().y - gap, left, right), (rect.center().y + gap, right, left)]
    {
        painter.line_segment([egui::pos2(tail, y), egui::pos2(tip, y)], stroke);
        let back = tip + (tail - tip).signum() * head;
        painter.add(egui::Shape::convex_polygon(
            vec![egui::pos2(tip, y), egui::pos2(back, y - half), egui::pos2(back, y + half)],
            color,
            egui::Stroke::NONE,
        ));
    }
}

/// The `L*` axis a brightness pair stands on, both ends included: 0 is black
/// and 100 is white, and a gradient is allowed to sit on either — flat, since a
/// ramp there has nowhere to open.
const L_STAR_AXIS: (f32, f32) = (0.0, 100.0);

/// The axis a chroma pair stands on: a FRACTION rather than a chroma, 0 grey
/// and 1 as vivid as the screen goes there. Not a share of this hue's own
/// ceiling — denominating in that ceiling is what made one setting read as
/// three different colorfulnesses around the circle. It is denominated in the
/// floor EVERY hue can hold and reaches this hue's own ceiling only at 1, so
/// one number is one colorfulness wherever the arc passes. See
/// [`Gradient::chroma`] for why the axis is a fraction rather than a chroma,
/// and `chroma_of` in `harmonigraph-scene` for the curve joining the two ends.
/// Both ends are settings and a pair on either is flat, exactly as a brightness
/// pair parked on black is — see [`Gradient::chroma`] for why the axis is
/// a fraction of what is available rather than a chroma.
const CHROMA_AXIS: (f32, f32) = (0.0, 1.0);

/// Which of the gradient's two stretches a [`SpreadBar`] is a bar of.
///
/// They are one control with two settings rather than two controls that
/// resemble each other, and the gradient is what makes them so: each is a
/// middle and a SIGNED ramp about it, bounded by what that middle leaves on its
/// own axis, with the two ends at `middle ± ramp/2`. What differs is the axis
/// itself and how a number on it is spelled — everything below is those two
/// answers, and nothing else varies between the bars.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Spread {
    /// `L*`, black to white.
    Brightness,
    /// One colorfulness whatever hue it lands on, grey to as vivid as it goes.
    Chroma,
}

impl Spread {
    /// The name the row carries. On the [`Spread`] rather than handed in: a bar
    /// can only be one of two things, and a label passed separately is a way for
    /// a row to name the other one.
    fn label(self) -> &'static str {
        match self {
            Spread::Brightness => "Brightness",
            Spread::Chroma => "Saturation",
        }
    }

    /// Both ends of the axis, in the units the gradient stores.
    fn axis(self) -> (f32, f32) {
        match self {
            Spread::Brightness => L_STAR_AXIS,
            Spread::Chroma => CHROMA_AXIS,
        }
    }

    /// Readout units per stored unit: an `L*` reads out as itself, a chroma
    /// fraction as a percentage of the color available where it is drawn.
    ///
    /// Also the SNAP grid, one readout unit of it, so that a drag lands both
    /// ends on numbers the readout can say exactly — see [`Self::snapped`].
    fn per_unit(self) -> f32 {
        match self {
            Spread::Brightness => 1.0,
            Spread::Chroma => 100.0,
        }
    }

    /// What follows each number in the readout: the sign a percentage is
    /// spelled with. Lightness counts perceptual black-to-white percent;
    /// saturation counts the fraction of the available chroma.
    fn suffix(self) -> &'static str {
        match self {
            Spread::Brightness => "%",
            Spread::Chroma => "%",
        }
    }

    /// The pair as the gradient holds it: a middle and a signed ramp.
    fn of(self, g: Gradient) -> (f32, f32) {
        match self {
            Spread::Brightness => (g.lightness, g.lightness_ramp),
            Spread::Chroma => (g.chroma, g.chroma_ramp),
        }
    }

    fn set(self, g: &mut Gradient, pair: (f32, f32)) {
        match self {
            Spread::Brightness => (g.lightness, g.lightness_ramp) = pair,
            Spread::Chroma => (g.chroma, g.chroma_ramp) = pair,
        }
    }

    /// The pair a bar actually writes, snapped at the ENDS rather than at the
    /// pair itself: both ends on a whole readout unit and inside the axis,
    /// which is what the readout says of them, and a readout is worth nothing
    /// once it is not the number the picture draws. The middle then lands on a
    /// whole or a half — 41 and 86 are a perfectly good pair of ends, and their
    /// middle is 63.5.
    ///
    /// Snapping the pair instead is the version that cannot be honest: a whole
    /// middle and a whole ramp of 45 reaches 41.5 and 86.5, which no rounding of
    /// the readout can say without lying half a point at both ends.
    ///
    /// Clamping the ends is also all the axis needs: a `Gradient` accepts a
    /// ramp as wide as its middle leaves it, and both ends inside the axis is
    /// the same statement made about the same two numbers — in exact arithmetic,
    /// which is what [`Self::legal`] is for.
    fn snapped(self, (centre, spread): (f32, f32)) -> (f32, f32) {
        let (min, max) = self.axis();
        let unit = self.per_unit();
        let end = |v: f32| ((v * unit).round() / unit).clamp(min, max);
        let (low, high) = (end(centre - spread * 0.5), end(centre + spread * 0.5));
        ((low + high) * 0.5, high - low)
    }

    /// The pair as [`Gradient::sanitized`] leaves it — asked of the
    /// gradient rather than restated here, which is the whole of how a bar and
    /// the type it writes to are kept from disagreeing about which pairs are
    /// legal.
    ///
    /// The last step of a write, and not a formality. [`Self::snapped`] puts
    /// both ENDS on the axis where the gradient bounds the RAMP by what its
    /// middle leaves: the same statement in exact arithmetic, and not quite the
    /// same one in f32 once the axis is a fraction. Whole `L*` recomposes
    /// exactly, so this moves nothing a brightness bar writes — none of the
    /// 10201 whole-point end pairs. A hundredth is no binary fraction, so 42 of
    /// the same 10201 chroma pairs recompose to a ramp past what their own
    /// middle holds — every one of them by exactly one ulp, 6e-8, `7%..100%`
    /// the first — and a bar writing one would leave the gradient drawing a
    /// picture off the pair the bar reads out.
    fn legal(self, pair: (f32, f32)) -> (f32, f32) {
        let mut g = Gradient::default();
        self.set(&mut g, pair);
        self.of(g.sanitized())
    }

    /// The two ends the ramp reaches, in PITCH order: the bottom of the pitch
    /// range first, whatever it happens to carry.
    ///
    /// Concrete where a middle and a signed ramp are arithmetic — these are the
    /// numbers the lowest and highest notes are actually drawn at, and they name
    /// the two handles standing under them. It is also how the sign gets said:
    /// an inverted ramp reads out backwards, high to low, where a signed number
    /// leaves the reader to work out which end it means.
    ///
    /// A tenth of a readout unit where an end is not whole, and no decimal where
    /// it is. Whole is what a drag leaves, since [`Self::snapped`] puts both ends
    /// there — but a fresh view, a double-click and a saved blob all arrive
    /// without passing it, and `ViewConfig`'s own gradient is one of them: 53
    /// over a ramp of 31 stands its ends on 37.5 and 68.5. Spelled to the whole
    /// point those read `38 → 68`, a span of 30 over a gradient that spends 31 —
    /// the readout claiming a picture the bar is not drawing, which is the one
    /// thing it cannot do and stay worth reading.
    ///
    /// Rounded to that tenth BEFORE being asked whether it is whole, which is
    /// what keeps a snapped chroma end from reading `42.0%`: a hundredth is no
    /// binary fraction, so an end snapped to 0.42 is 41.999998 percent of the way
    /// up its axis, and 42 is both what it means and what a tenth of a unit can
    /// say.
    fn readout(self, (centre, spread): (f32, f32)) -> String {
        let end = |v: f32| {
            let v = (v * self.per_unit() * 10.0).round() / 10.0;
            let n = if v == v.round() { format!("{v:.0}") } else { format!("{v:.1}") };
            format!("{n}{}", self.suffix())
        };
        format!("{} \u{2192} {}", end(centre - spread * 0.5), end(centre + spread * 0.5))
    }

    /// The widest the readout goes, for the reserve the name is elided against.
    /// Only its LENGTH matters, the numbers being monospace: three digits and a
    /// tenth at each end, plus whatever follows them. No end can carry a sign,
    /// both of them living on an axis that starts at 0.
    ///
    /// Built from the axis rather than written out, so a bar cannot be added
    /// with a reserve measured for another one's numbers.
    fn widest_readout(self) -> String {
        let (end, suffix) = (self.axis().1 * self.per_unit(), self.suffix());
        format!("{end:.1}{suffix} \u{2192} {end:.1}{suffix}")
    }
}

/// Which part of a [`SpreadBar`] a drag took hold of. The same three a
/// [`Grab`] names, decided on the first frame of the gesture and remembered for
/// the rest of it for the same reason — and the memory earns more here, because
/// these two ends may CROSS: an end dragged past its partner swaps which side
/// of the bar it stands on, so "the handle nearest the pointer" names the other
/// end by the next frame.
///
/// The ends are named for the pitch they carry rather than for where they
/// stand: [`Low`](SpreadGrab::Low) is the bottom of the pitch range, which is
/// the left-hand handle at a positive ramp and the right-hand one at a negative.
///
/// (`Default` is derived only to satisfy egui's `remove_temp` bound; the value
/// is always written by drag-start before anything reads it.)
///
/// [`Grab`]: super::range::Grab
#[derive(Clone, Copy, Debug, Default)]
enum SpreadGrab {
    #[default]
    Low,
    High,
    /// The ramp itself, sliding along the axis at a fixed width: `offset` is how
    /// far from the middle the pointer took hold and `spread` how wide the ramp
    /// was at that moment, both fixed for the gesture. [`Grab::Span`] fixes its
    /// own two for the same reason, and the squish below is the same bargain.
    ///
    /// [`Grab::Span`]: super::range::Grab::Span
    Middle {
        offset: f32,
        spread: f32,
    },
}

impl SpreadGrab {
    /// What a drag starting at value `v` takes hold of: an end if the pointer is
    /// near one, the ramp if it is inside, otherwise the nearer end. A [`Grab`]
    /// divides a bar the same way, and the share of the ramp a handle's reach
    /// may claim is the same constant.
    ///
    /// [`Grab`]: super::range::Grab
    fn at(v: f32, (centre, spread): (f32, f32), near: f32) -> SpreadGrab {
        let (low_end, high_end) = (centre - spread * 0.5, centre + spread * 0.5);
        let (d_low, d_high) = ((v - low_end).abs(), (v - high_end).abs());
        let nearer = if d_low == d_high {
            // A tie is the FLAT ramp — the two ends stand on the same point, so
            // which one a press takes is a rule rather than a measurement. Take
            // the end on the side the pointer is, and the ramp opens the way it
            // is dragged: up lifts the top of the pitch range, down darkens the
            // bottom, and neither leaves the picture upside down. A fixed
            // choice inverts it in whichever direction it is not — and parked
            // on black or white that is the ONLY direction, so the right way
            // round would be unreachable from there.
            if v < centre {
                SpreadGrab::Low
            } else {
                SpreadGrab::High
            }
        } else if d_low < d_high {
            SpreadGrab::Low
        } else {
            SpreadGrab::High
        };
        // A handle's reach cannot eat the whole ramp, or a narrow one would
        // have no middle left to slide along the axis.
        let reach = near.min(spread.abs() * HANDLE_REACH_SHARE);
        // And when the ramp is too narrow for the ends to have room of their
        // own, the MIDDLE takes a full reach instead — at a flat ramp all three
        // stand on one point, and a bar that could not move brightness at
        // exactly the isoluminant setting would strand anyone who dialled their
        // way into it. This is the mirror of [`Grab::at`]'s own fallback, which
        // hands a span with nowhere to slide to the nearer end.
        if reach < near {
            return if (v - centre).abs() <= near {
                SpreadGrab::Middle { offset: v - centre, spread }
            } else {
                nearer
            };
        }
        if d_low.min(d_high) <= reach {
            nearer
        } else if v > low_end.min(high_end) && v < low_end.max(high_end) {
            SpreadGrab::Middle { offset: v - centre, spread }
        } else {
            nearer
        }
    }

    /// Where the pair ends up when this grab is dragged to value `v`. Pure, so
    /// what actually matters — both ends stay on the axis, an end moves without
    /// disturbing its partner, and an end dragged past that partner inverts the
    /// ramp rather than stopping against it — is testable without a pointer.
    ///
    /// An end drag reads the pair back to find the end it is NOT moving, as a
    /// [`Grab`] reads its own partner; a middle drag reads neither, working from
    /// the width and offset its own gesture began at. Neither reads back a
    /// number it is itself writing, which is what keeps a drag from creeping
    /// while the pointer sits still.
    ///
    /// [`Grab`]: super::range::Grab
    fn apply(self, v: f32, (centre, spread): (f32, f32), (min, max): (f32, f32)) -> (f32, f32) {
        // The pair, as the two ends it draws — which is what the gestures below
        // are actually about, and what the readout says.
        let (low_end, high_end) = (centre - spread * 0.5, centre + spread * 0.5);
        let pair = |low: f32, high: f32| ((low + high) * 0.5, high - low);
        match self {
            // One end to the pointer, its partner untouched. Past that partner
            // the ramp INVERTS rather than stopping there — the gesture keeps
            // hold of the end it grabbed, so the two simply trade sides, and
            // that is the whole of how the bright end gets to the bottom of the
            // pitch range. A [`RangeBar`] forbids exactly this, and is right to:
            // its ends bound a pitch axis, which inverted maps every pitch on it
            // backwards.
            SpreadGrab::Low => pair(v.clamp(min, max), high_end),
            SpreadGrab::High => pair(low_end, v.clamp(min, max)),
            SpreadGrab::Middle { offset, spread } => {
                let half = spread.abs() * 0.5;
                let want = v - offset;
                // Against a wall the ramp squishes rather than the drag jamming,
                // the bargain [`Grab::Span`] makes: the leading end pins and the
                // trailing one carries on with the pointer, so brightness
                // dragged toward white keeps moving instead of stopping dead.
                // Reading the width the GESTURE began with rather than the
                // squished one it just wrote is what opens it back out on the
                // way home.
                let (lo, hi) = if want - half < min {
                    (min, (want + half).clamp(min, max))
                } else if want + half > max {
                    ((want - half).clamp(min, max), max)
                } else {
                    (want - half, want + half)
                };
                // Squishing changes the ramp's width, never its direction.
                let (centre, width) = pair(lo, hi);
                (centre, width.copysign(spread))
            }
        }
    }
}

/// The stretch of an axis the pitch range spends: a two-ended bar whose ends
/// ARE the gradient's ends, the bottom of the pitch range and the top.
///
/// One bar for the gradient's two stretches, brightness and chroma, because
/// they are one thing set twice (see [`Spread`]). Drag either end to move it,
/// drag between them to slide the ramp at a fixed width, drag one end past the
/// other to swap which end of the pitch range carries the most, and
/// double-click to reset.
///
/// **A [`RangeBar`] in behaviour, and two things apart from it.** The ends may
/// cross, because crossed is a real setting here and not a broken one — it is
/// the inverted picture — where a range bar's ends bound a pitch axis that
/// inverted maps every pitch backwards. And it writes a MIDDLE and a signed
/// ramp rather than the pair it draws, because that is what a gradient holds:
/// a value at the centre of the pitch range and a signed difference between its
/// ends, so the ends are `middle ± ramp/2` and the two shapes carry exactly
/// the same information. What that buys the pane is a row: a bar per number
/// names the same two numbers and draws neither the stretch they compose nor
/// the room the axis has left for it (see `spectrum_group`).
///
/// **Nothing marks the middle**, though it is the number the gradient stores.
/// It is not a thing a gesture takes hold of — the slide takes the whole ramp —
/// and a mark on a two-ended bar reads as a third handle whatever it is drawn
/// like. The two ends are what the picture is made of and what the readout
/// says; the middle is where they happen to average.
///
/// **The readout is the two ENDS, and it runs in pitch order.** They are what
/// the picture concretely does — the `L*` the darkest and brightest notes are
/// drawn at, the color the palest and most vivid ones carry — and each of them
/// names a handle standing under it, where a centre and a signed ramp name
/// neither. Pitch order is also the only place the SIGN can live: a ramp and
/// its negative put the two handles in exactly the same places, so the bar
/// cannot draw the difference, and an inverted ramp reads out backwards
/// instead, high to low. (What the sign means for the picture is at the top of
/// the group, on the [`GradientPreview`], which draws the gradient in pitch
/// order and so reverses with it.)
///
/// **Both ends stay on the axis at every setting.** That is the bar's own
/// geometry — a handle off the track is not a value it can express — and
/// [`Gradient::sanitized`] holds the same line for a pair that arrives
/// from a hand-edited file instead of through a gesture.
/// `the_bar_can_only_reach_pairs_sanitize_leaves_alone` is what keeps the two
/// from drifting into disagreeing about which pairs are legal, and
/// [`Spread::legal`] is how a write earns it.
///
/// **A thumb reaches that readout**, which is the price of parking one run at
/// the right rather than placing two the way a [`RangeBar`] does. Neither run
/// here can dodge — the name is pinned left, the readout parked right — so both
/// are knocked out through the thumbs by [`grip_over_text`] and a crossed digit
/// inverts rather than disappearing.
///
/// Whether a thumb RESTS in a run is decided by the pairs the defaults open
/// with, which a capture retunes freely, so no width is promised here.
/// `the_bars_the_panes_build_are_knocked_out_wherever_they_rest_under_a_thumb`
/// sweeps the four bars the panes build at those pairs and holds the knockout
/// wherever one does, without pinning where. A high end past about four fifths
/// of its axis stands in the readout at the width the settings column opens
/// at, so a fresh look can lean on the knockout as well as a drag.
///
/// [`RangeBar`]: super::range::RangeBar
/// [`grip_over_text`]: super::bar::grip_over_text
pub struct SpreadBar<'a> {
    gradient: &'a mut Gradient,
    spread: Spread,
    home: Gradient,
}

/// Both constructors take the gradient a double-click takes this bar's PAIR home
/// to — its own stretch of it, the other left alone. See [`SpectrumBar::new`] for
/// why every caller names it.
impl<'a> SpreadBar<'a> {
    /// The `L*` the bottom and the top of the range are drawn at.
    pub fn brightness(gradient: &'a mut Gradient, home: Gradient) -> Self {
        SpreadBar { gradient, spread: Spread::Brightness, home }
    }

    /// How much of the color available to them the bottom and the top of the
    /// range carry.
    pub fn chroma(gradient: &'a mut Gradient, home: Gradient) -> Self {
        SpreadBar { gradient, spread: Spread::Chroma, home }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let scale = theme::ui_scale(ui.ctx());
        let width = bar_width(ui);
        let (rect, mut response) = ui.allocate_exact_size(
            Vec2::new(width, theme::row_height(scale)),
            Sense::click_and_drag(),
        );
        let axis = self.spread.axis();
        let (min, max) = axis;
        // Values live on an inset track, so both limits are places a handle can
        // stand rather than edges it merges into. See HANDLE_INSET.
        let track = rect.shrink2(Vec2::new(HANDLE_INSET * scale, 0.0));
        let x_of =
            |v: f32| track.left() + track.width() * ((v - min) / (max - min)).clamp(0.0, 1.0);
        let value_at = |x: f32| {
            min + ((x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0) * (max - min)
        };
        let pair = |g: Gradient| self.spread.of(g);

        // ---- Interaction ----------------------------------------------------
        let near = GRAB_PX / track.width().max(1.0) * (max - min);
        // Reset rather than text entry, the bargain a [`RangeBar`] makes: a bar
        // holding two numbers has no single value to type into it.
        if response.double_clicked() {
            self.spread.set(self.gradient, self.spread.of(self.home.sanitized()));
            response.mark_changed();
        }
        let aimed = pair(self.gradient.sanitized());
        let held = drag(ui, &response, |aim| SpreadGrab::at(value_at(aim.x), aimed, near));
        if let Some((grab, p)) = held {
            let next =
                self.spread.legal(self.spread.snapped(grab.apply(value_at(p.x), aimed, axis)));
            if next != pair(*self.gradient) {
                self.spread.set(self.gradient, next);
                response.mark_changed();
            }
        }
        let holding = held.map(|(grab, _)| grab);

        // Off screen nothing below is needed; see the same line in `ValueBar`.
        if !ui.is_rect_visible(rect) {
            return response;
        }
        // ---- Paint ----------------------------------------------------------
        // The pair read BACK, not the one the gesture was aimed at: a drag has
        // just written it, and painting the earlier value leaves the handles a
        // whole frame behind the pointer. Every other bar here re-reads for the
        // same reason.
        let (centre, spread) = pair(self.gradient.sanitized());
        let (lo, hi) = (centre - spread.abs() * 0.5, centre + spread.abs() * 0.5);
        let radius = CornerRadius::same(bar_radius(scale));
        let painter = ui.painter();
        painter.rect_filled(rect, radius, theme::well());

        let fill_color = track_fill(&response);
        // The stretch of the axis the picture spends, which is what the pair
        // MEANS: a flat ramp fills nothing, and that is the honest drawing of a
        // gradient that spends none of this axis on pitch.
        let (lx, hx) = (x_of(lo), x_of(hi));
        let mut span = rect;
        span.min.x = lx;
        span.max.x = hx;
        painter.rect_filled(span, radius, fill_color);

        // Name and readout exactly as a ValueBar lays them out — the row is one
        // — with the same reserve trick: the width kept clear for the numbers
        // is measured off a string that never changes rather than off the pair
        // currently in the bar, so the name cannot re-elide mid-drag. See
        // [`Spread::widest_readout`] for what that string is.
        let text_color = theme::text();
        let mono = TextStyle::Monospace.resolve(ui.style());
        let value = painter.layout_no_wrap(
            self.spread.readout((centre, spread)),
            mono.clone(),
            theme::text(),
        );
        let reserve =
            painter.layout_no_wrap(self.spread.widest_readout(), mono, theme::text()).size().x;
        let body = TextStyle::Body.resolve(ui.style());
        let job = egui::text::LayoutJob::simple_singleline(
            self.spread.label().to_owned(),
            body,
            text_color,
        );
        let text_pad = BAR_TEXT_PAD * scale;
        let label = elided_name(painter, job, rect.width(), scale, reserve);
        let centered = |galley: &egui::Galley, x: f32| text_at(rect, galley, x);
        let label_pos = centered(&label, rect.left() + text_pad);
        let value_pos = centered(&value, rect.right() - text_pad - value.size().x);
        painter.galley(label_pos, label.clone(), text_color);
        painter.galley(value_pos, value.clone(), theme::text());

        // The handles on top of the text, a RangeBar's bargain: they are the
        // part you operate, and a digit sliding under one beats a handle
        // disappearing behind a digit. At a flat ramp the two coincide, and one
        // thumb standing on an empty track is the right picture — there is one
        // place the whole range is.
        //
        // BOTH runs are knocked out through the thumbs here, where a RangeBar
        // does it for its name alone, and the difference is that neither of
        // these can move. A RangeBar picks a run of clear track for each of its
        // numbers; this bar spells its two ends into ONE readout parked at the
        // right, which buys the pair a single run to read but stands it where a
        // handle taken past about four fifths of the axis arrives — see the
        // type's docs for what that means for a bar at rest.
        //
        // Lit by what is in hand (see [`grip_color`]). The ends are named for
        // the PITCH they carry, so which thumb is the low end is the ramp's
        // sign: the left one at a rising ramp, the right one at a falling. A
        // flat ramp stands both on one point, so the unlit one is painted first
        // and the lit one shows (`paint_thumbs`).
        let in_hand = holding.or_else(|| {
            poised(ui, &response).map(|p| SpreadGrab::at(value_at(p.x), (centre, spread), near))
        });
        let rising = spread >= 0.0;
        let (left_lit, right_lit) = match in_hand {
            Some(SpreadGrab::Low) => (rising, !rising),
            Some(SpreadGrab::High) => (!rising, rising),
            Some(SpreadGrab::Middle { .. }) | None => (true, true),
        };
        paint_thumbs(
            painter,
            rect,
            scale,
            grip_radius(scale),
            [(lx, left_lit), (hx, right_lit)],
            &[(label_pos, label), (value_pos, value)],
        );

        // The cursor says which gesture a press would start before committing
        // to it: a handle opens the ramp, the middle picks the whole thing up.
        // Read off the same grab the thumbs are lit by.
        match in_hand {
            Some(SpreadGrab::Middle { .. }) => response.on_hover_cursor(egui::CursorIcon::Grab),
            Some(_) => response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal),
            None => response,
        }
    }
}

#[cfg(test)]
mod tests;
