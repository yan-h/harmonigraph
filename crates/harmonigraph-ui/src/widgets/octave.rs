//! [`OctaveStrip`]: the octave wheel's two counts and the profile they produce,
//! in one row.

use egui::{CornerRadius, Response, Sense, TextStyle, Ui, Vec2};
use harmonigraph_scene::{
    clamp_wheel, octave_layout, ViewConfig, DEFAULT_CENTER, DEFAULT_COUNT, MAX_SPAN, MIN_SPAN,
};

use super::bar::{
    aimed_at, bar_radius, bar_width, elided_name, grabbed, release_grab, track_fill, BAR_TEXT_PAD,
};
use crate::theme;

/// Gap between two of the octave strip's cells, so a wheel reads as a row of
/// separate octaves rather than one fill with steps in it.
const CELL_GAP: f32 = 1.0;

/// Shortest an octave strip cell is ever drawn. The thinnest extra there is
/// would come out under a pixel on a short bar, and a cell that is not there
/// says the octave is not either.
const CELL_MIN_H: f32 = 3.0;

/// Width of an octave strip's handles. Narrower than a [`RangeBar`]'s, because
/// this one sits ON a boundary between two cells and hides a slice of each,
/// and a slot is only a eleventh of the bar to begin with — but not under the
/// four points a handle needs to read as something to grab rather than as an
/// edge in the fill.
///
/// [`RangeBar`]: super::range::RangeBar
const STRIP_HANDLE_W: f32 = 4.0;

/// Which of the two counts a drag on the octave strip took hold of, decided on
/// the first frame of the gesture and remembered for the rest of it.
///
/// `Default` only because egui's temp-data store demands it of anything it can
/// remove; nothing reads the default, since the value is always written by
/// drag-start first.
///
/// BOTH variants carry the other count, so `apply` is a pure function of how
/// far out the pointer is and the gesture cannot read back a number it moved
/// itself. Each direction has its own way of moving one: raising the count
/// past the eleven-slice budget makes the extras yield, and taking the fringe
/// off a lone full-size octave opens the count to two (`clamp_wheel`'s answer
/// for an undrawable wheel). Either one read back mid-gesture makes dragging
/// out and home again a one-way trip.
#[derive(Clone, Copy)]
enum StripGrab {
    /// The full-size octaves, measured from the middle of the wheel.
    Count { extras: u32 },
    /// The fringe, measured from the edge of the count — so `count` is the
    /// gesture's own zero point as well as the number it must not move.
    Extras { count: u32 },
}

impl Default for StripGrab {
    fn default() -> Self {
        StripGrab::Extras { count: DEFAULT_COUNT }
    }
}

impl StripGrab {
    /// What a drag starting `reach` slots out from the middle of the wheel
    /// takes hold of: the full-size octaves while it is inside the handles,
    /// the fringe outside them.
    ///
    /// By REGION rather than by the nearer handle, which is what keeps the two
    /// apart at zero extras — there both handles sit on the wheel's outer edge
    /// and every press is equally near one, while the gesture a press there
    /// wants is unambiguous: outward is a fringe, inward is a count.
    fn at(reach: f32, count: u32, extras: u32) -> StripGrab {
        if reach <= count as f32 * 0.5 {
            StripGrab::Count { extras }
        } else {
            StripGrab::Extras { count }
        }
    }

    /// Where the two counts end up when this grab is dragged `reach` slots out
    /// from the middle. Pure, and a function of `reach` ALONE, so the things
    /// that actually matter — half a slot of travel per octave, a fringe
    /// measured from the edge of the count, a wheel that never overruns the
    /// budget, and a drag that comes home to where it started — are testable
    /// without a pointer.
    fn apply(self, reach: f32) -> (u32, u32) {
        // Half a slot per octave in both gestures: the count grows at both
        // ends at once, and so does the fringe.
        let (count, extras) = match self {
            StripGrab::Count { extras } => ((2.0 * reach).round() as u32, extras),
            StripGrab::Extras { count } => {
                let want = (reach - count as f32 * 0.5).max(0.0).round() as u32;
                // A lone full-size octave is only drawable with a pair to
                // flank it. Opening the count to two is what `clamp_wheel`
                // does about that, which is the right answer for a blob and
                // the wrong one for a gesture that is only holding the fringe:
                // the fringe stops at one instead, and the count stays where
                // the other gesture left it.
                let floor = if count < MIN_SPAN { 1 } else { 0 };
                (count, want.max(floor))
            }
        };
        clamp_wheel(count, extras)
    }
}

/// The octave wheel's two counts as one control: a strip of eleven slots — the
/// whole budget, since a wheel past eleven slices is one the boundary table
/// cannot hold — with the wheel drawn centered in it. The cells between the
/// two handles are the full-size octaves, the ones outside are the extras at
/// each end, and the empty track past those is the budget still unspent.
///
/// Drag inside the handles to set the count, outside them to set the extras;
/// double-click goes home to [`reset_wheel`]. Which one a drag takes is decided
/// by where it STARTS rather than by proximity to a handle, so the gesture
/// cannot change its mind halfway — and so the two are still told apart at
/// zero extras, where both handles sit on the wheel's outer edge and a
/// nearest-handle rule would have nothing to say.
///
/// Cell WIDTH is a slot of the budget; cell HEIGHT is how much of the ring
/// that octave takes AGAINST THE WIDEST ONE ON THIS WHEEL, so the full-size
/// octaves stand full height at every count and the axis reads "of a full-size
/// octave" rather than "of a turn". Two wheels cannot be compared by height —
/// a lone octave and one of eleven both draw full — and that is the right
/// trade, since against the turn the strip would spend its whole height on the
/// count and leave the fringe, which is what the bars below actually set, in
/// the bottom tenth.
///
/// Carrying the profile at all is the whole reason this is a strip and not two
/// bars: the fringe's size and blend have nowhere else to be seen before they
/// are dragged, and the thing they trade against — how much of the turn the
/// full-size octaves keep — is the same picture read the other way.
pub struct OctaveStrip<'a> {
    count: &'a mut u32,
    extras: &'a mut u32,
    /// The fringe knobs, read-only: they shape the profile the strip draws and
    /// have their own bars under it.
    size: f32,
    blend: f32,
}

/// The wheel a double-click on the strip goes home to: the one a fresh view
/// opens with.
///
/// Read off [`ViewConfig::default`] rather than restated as a literal, because
/// a reset that names its own pair drifts the moment the fresh wheel moves —
/// and it silently, since the strip goes on resetting to a wheel that is
/// merely no longer anyone's default. Landing on zero extras is the visible
/// end of that: the `Extra size` and `Extra blend` bars gate on there being a
/// fringe, so they gray out still holding the values the reset just orphaned.
pub(super) fn reset_wheel() -> (u32, u32) {
    let fresh = ViewConfig::default();
    (fresh.octave_count, fresh.octave_extras)
}

impl<'a> OctaveStrip<'a> {
    pub fn new(count: &'a mut u32, extras: &'a mut u32, size: f32, blend: f32) -> Self {
        OctaveStrip { count, extras, size, blend }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let scale = theme::ui_scale(ui.ctx());
        let width = bar_width(ui);
        let (rect, mut response) = ui.allocate_exact_size(
            Vec2::new(width, theme::row_height(scale)),
            Sense::click_and_drag(),
        );
        let slot = (rect.width() / MAX_SPAN as f32).max(1.0);
        let middle = rect.center().x;
        // How far from the middle of the wheel a point is, in slots — the one
        // measure both gestures are written in, since the wheel is symmetric
        // and both counts grow from its middle outward.
        let out = |x: f32| (x - middle).abs() / slot;

        // ---- Interaction ----------------------------------------------------
        let grab_id = response.id.with("grab");
        if response.double_clicked() {
            (*self.count, *self.extras) = reset_wheel();
            response.mark_changed();
        }
        if response.dragged() {
            if let Some(p) = response.interact_pointer_pos() {
                let reach = out(p.x);
                let grab = grabbed(ui, grab_id, |ui| {
                    // From where the press LANDED (see `aimed_at`), which
                    // this control needs more than the bars with handles do
                    // and not differently: it splits its two gestures on a
                    // hard line rather than on a reach, and half of the
                    // drawn handle sits inside the six points egui spends
                    // deciding a press is a drag — so the live position
                    // hands "grab the handle, pull it outward", which is the
                    // count, to the fringe.
                    let start = out(aimed_at(ui, p).x);
                    StripGrab::at(start, *self.count, *self.extras)
                });
                let (count, extras) = grab.apply(reach);
                if (count, extras) != (*self.count, *self.extras) {
                    (*self.count, *self.extras) = (count, extras);
                    response.mark_changed();
                }
            }
        }
        if response.drag_stopped() {
            release_grab::<StripGrab>(ui, grab_id);
        }

        // ---- Paint ----------------------------------------------------------
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(bar_radius(scale)), theme::well());

        let fill_color = track_fill(&response);
        // The widths the wheel actually comes out at. The center pitch turns
        // each node's ring but never touches the widths, so which one this
        // asks for cannot show.
        let wheel = octave_layout(*self.count, DEFAULT_CENTER, *self.extras, self.size, self.blend);
        let span = wheel.span as usize;
        let cell_width = |i: usize| wheel.bounds[i + 1] - wheel.bounds[i];
        // Against the widest rather than against a whole turn, so the full-size
        // octaves stand at full height at every count and the strip's vertical
        // axis reads as "of a full-size octave".
        let widest = (0..span).map(cell_width).fold(0.0f32, f32::max).max(1e-6);
        let left = middle - 0.5 * span as f32 * slot;
        let gap = CELL_GAP * scale;
        let cell_radius = CornerRadius::same(theme::scaled_points(2, scale));
        for i in 0..span {
            let height = (rect.height() * cell_width(i) / widest).max(CELL_MIN_H * scale);
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(left + i as f32 * slot + 0.5 * gap, rect.bottom() - height),
                    egui::pos2(left + (i + 1) as f32 * slot - 0.5 * gap, rect.bottom()),
                ),
                cell_radius,
                fill_color,
            );
        }

        // Where the count ends and the fringe begins, which is also where a
        // drag changes its meaning — drawn whether or not there are extras
        // yet, since that is the edge you drag OUT from to get some.
        let handle_w = STRIP_HANDLE_W * scale;
        // Held inside the bar by half a handle, for the reason HANDLE_INSET
        // holds a RangeBar's ends off theirs: a wheel that spends the whole
        // budget puts this boundary on the bar's own edge, and a handle
        // centered there hangs half its width out over the pane, where the
        // part still on the bar reads as its border rather than as something
        // to grab. There is nothing outside it to drag toward at that width
        // anyway — a full count leaves no room for a fringe.
        let inset = 0.5 * handle_w;
        for side in [-1.0f32, 1.0] {
            let x = (middle + side * *self.count as f32 * 0.5 * slot)
                .clamp(rect.left() + inset, rect.right() - inset);
            painter.rect_filled(
                egui::Rect::from_center_size(
                    egui::pos2(x, rect.center().y),
                    Vec2::new(handle_w, rect.height() - 3.0 * scale),
                ),
                cell_radius,
                theme::text(),
            );
        }

        // Name and readout as a ValueBar wears them. The readout is the wheel
        // spelled out — the fringe, the count, the fringe — because the number
        // that matters depends on which of them is being dragged, and their
        // sum is what the eleven-slot budget is against.
        let text_color = if response.hovered() || response.dragged() {
            theme::text()
        } else {
            theme::text_dim()
        };
        let mono = TextStyle::Monospace.resolve(ui.style());
        let shown = if *self.extras > 0 {
            format!("{}+{}+{}", self.extras, self.count, self.extras)
        } else {
            format!("{}", self.count)
        };
        let value = painter.layout_no_wrap(shown, mono.clone(), theme::text());
        // Room kept clear for the widest readout the strip can produce rather
        // than for the one in it, so the name does not re-elide as the wheel
        // gains a digit mid-drag. Monospace, so any five characters measure
        // the same and a count of eleven (which can carry no extras) is
        // shorter than every fringed wheel there is.
        let reserve = painter.layout_no_wrap("0+0+0".into(), mono, theme::text()).size().x;
        let body = TextStyle::Body.resolve(ui.style());
        let mut job = egui::text::LayoutJob::default();
        job.append("Octaves", 0.0, egui::TextFormat::simple(body, text_color));
        let text_pad = BAR_TEXT_PAD * scale;
        let label = elided_name(painter, job, rect.width(), scale, reserve);
        let centered =
            |galley: &egui::Galley, x: f32| egui::pos2(x, rect.center().y - galley.size().y * 0.5);
        painter.galley(centered(&label, rect.left() + text_pad), label, text_color);
        painter.galley(
            centered(&value, rect.right() - text_pad - value.size().x),
            value,
            theme::text(),
        );

        response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
    }
}

#[cfg(test)]
mod tests {
    use harmonigraph_scene::{DEFAULT_EXTRA_SIZE, MIN_EXTRA_SIZE};

    use super::*;
    use crate::widgets::probe::{filled_rects, handles, shapes, text_boxes};

    /// Double-clicking the strip is the only way back to the stock wheel, so
    /// where it lands has to BE the stock wheel — not a pair that was the
    /// stock wheel when the gesture was written. The fringe is the half that
    /// goes wrong quietly: reset to zero extras and the two bars under the
    /// strip gray out holding a size and a blend nothing is drawing, which
    /// reads as the fringe knobs being unavailable rather than as the reset
    /// having thrown the fringe away.
    #[test]
    fn a_double_click_goes_home_to_the_wheel_a_fresh_view_opens_with() {
        let fresh = ViewConfig::default();
        assert_eq!(reset_wheel(), (fresh.octave_count, fresh.octave_extras));
        assert!(
            reset_wheel().1 > 0,
            "the stock wheel carries a fringe, so the reset must leave the \
             Extra size and Extra blend bars live",
        );
    }

    /// Paint one octave strip across a 300pt row and return what it emitted.
    fn paint_octave_strip(count: u32, extras: u32, size: f32, blend: f32) -> Vec<egui::Shape> {
        let (mut c, mut e) = (count, extras);
        shapes(300.0, |ui| {
            OctaveStrip::new(&mut c, &mut e, size, blend).show(ui);
        })
    }

    /// The strip's cells, left to right: the accent-filled rects, which the
    /// well behind them and the two handles over them are not.
    fn cells(shapes: &[egui::Shape]) -> Vec<egui::Rect> {
        let mut cells: Vec<_> = filled_rects(shapes)
            .into_iter()
            .filter(|(_, fill)| *fill == theme::accent_fill())
            .map(|(r, _)| r)
            .collect();
        cells.sort_by(|a, b| a.left().total_cmp(&b.left()));
        cells
    }

    /// One cell per octave the wheel draws, and its HEIGHT is the share of the
    /// ring that octave takes — which is the whole of what the strip says that
    /// two count bars could not.
    #[test]
    fn the_strip_draws_one_cell_per_octave_at_its_own_width() {
        // An even wheel: every octave the same, and the cells with it.
        let shapes = paint_octave_strip(5, 0, DEFAULT_EXTRA_SIZE, 0.0);
        let bar = filled_rects(&shapes)[0].0;
        let even = cells(&shapes);
        assert_eq!(even.len(), 5, "one cell per octave");
        for cell in &even {
            assert!(
                (cell.height() - even[0].height()).abs() < 0.01,
                "an even wheel drew cells of different heights",
            );
            // Against the widest octave on the wheel, so a full-size one is
            // the whole row — the scale, not just the ordering, which nothing
            // else here would notice.
            assert!(
                (cell.height() - bar.height()).abs() < 0.01,
                "a full-size octave is {} of the row, not all of it",
                cell.height() / bar.height()
            );
        }
        // A flat fringe: two tiers, the extras equal and shorter, symmetric.
        let flat = cells(&paint_octave_strip(3, 2, 0.4, 0.0));
        assert_eq!(flat.len(), 7, "three full-size octaves and two extras each end");
        let (extra, full) = (flat[0].height(), flat[2].height());
        assert!(extra < full, "the extras are not shorter than the full-size octaves");
        assert!((flat[1].height() - extra).abs() < 0.01, "a flat fringe is not flat");
        assert!((flat[6].height() - extra).abs() < 0.01, "the fringe is lopsided");
        assert!((flat[3].height() - full).abs() < 0.01, "the full-size octaves differ");
        // A graded one: the inner extra stands between the two tiers.
        let ramp = cells(&paint_octave_strip(3, 2, 0.4, 1.0));
        assert!(
            ramp[0].height() < ramp[1].height() && ramp[1].height() < ramp[2].height(),
            "the blend did not grade the fringe: {:?}",
            ramp.iter().map(egui::Rect::height).collect::<Vec<_>>()
        );
    }

    /// The thinnest extra there is comes out under a pixel of a 20pt row, and
    /// a cell that is not there says the octave is not either — which is what
    /// `CELL_MIN_H` exists to stop. No other fixture reaches it: the floor
    /// only binds under about a seventh of a full-size octave, and every other
    /// strip painted here sits well above that.
    #[test]
    fn the_thinnest_extra_still_draws() {
        let shapes = paint_octave_strip(5, 3, MIN_EXTRA_SIZE, 0.0);
        let bar = filled_rects(&shapes)[0].0;
        let cells = cells(&shapes);
        assert_eq!(cells.len(), 11, "five full-size octaves and three extras each end");
        // The fixture has to actually reach the floor, or this passes on a
        // cell the clamp never touched. Unclamped it is 0.1 of an even slice
        // against a full-size octave of 2.08 — 0.96pt of a 20pt row.
        let wheel = octave_layout(5, DEFAULT_CENTER, 3, MIN_EXTRA_SIZE, 0.0);
        let ratio = (wheel.bounds[1] - wheel.bounds[0]) / (wheel.bounds[4] - wheel.bounds[3]);
        let unclamped = ratio * bar.height();
        assert!(unclamped < CELL_MIN_H, "the fixture does not reach the floor: {unclamped}pt");
        let extra = cells[0].height();
        assert!((extra - CELL_MIN_H).abs() < 0.01, "the thinnest extra drew {extra}pt");
    }

    /// What the strip says in words. The readout carries three numbers where
    /// every other bar in the pane carries one, and their ORDER is the whole
    /// of what tells a fringed wheel from its transpose — "2+5+2" and "5+2+5"
    /// are both plausible-looking readouts for the same three digits.
    #[test]
    fn the_strip_reads_out_the_wheel_it_draws() {
        let texts = |shapes: &[egui::Shape]| {
            text_boxes(shapes).into_iter().map(|(_, t)| t).collect::<Vec<_>>()
        };
        assert_eq!(
            texts(&paint_octave_strip(5, 2, 0.4, 0.0)),
            vec!["Octaves".to_owned(), "2+5+2".to_owned()],
            "a fringed wheel reads out fringe, count, fringe"
        );
        // No fringe, no plus signs: a bare count, so the readout says there is
        // nothing outside the handles rather than spelling out a zero.
        assert_eq!(
            texts(&paint_octave_strip(MAX_SPAN, 0, 0.4, 0.0)),
            vec!["Octaves".to_owned(), "11".to_owned()],
            "an unfringed wheel reads out its count alone"
        );
    }

    /// The wheel sits centered in the eleven slots, so the empty track at each
    /// end is what is left of the budget — and a slot is the same width
    /// whatever the wheel, which is what makes the strip a fixed axis to drag
    /// on rather than one that stretches under the pointer.
    #[test]
    fn the_strips_slots_are_a_fixed_axis_the_wheel_is_centered_on() {
        let bar = filled_rects(&paint_octave_strip(5, 0, DEFAULT_EXTRA_SIZE, 0.0))[0].0;
        let slot = bar.width() / MAX_SPAN as f32;
        for (count, extras) in [(5u32, 0u32), (5, 3), (11, 0), (1, 1)] {
            let drawn_cells = cells(&paint_octave_strip(count, extras, 0.4, 0.0));
            let span = count + 2 * extras;
            assert_eq!(drawn_cells.len(), span as usize, "{count}+2x{extras}: wrong cell count");
            let drawn = drawn_cells[drawn_cells.len() - 1].right() - drawn_cells[0].left();
            assert!(
                (drawn - span as f32 * slot).abs() < 1.5,
                "{count}+2x{extras} spans {drawn} of the {} its slots are worth",
                span as f32 * slot,
            );
            let middle =
                0.5 * (drawn_cells[0].left() + drawn_cells[drawn_cells.len() - 1].right());
            assert!((middle - bar.center().x).abs() < 0.5, "{count}+2x{extras} is off center");
        }
    }

    /// The handles are what say the strip has two gestures at all, and at zero
    /// extras — the state a fresh view is in — they are the only mark on it
    /// saying where one ends and the other begins. Same lesson the range bar
    /// learned: a handle under four points reads as an edge in the fill.
    #[test]
    fn the_strips_handles_sit_on_the_wheels_edges_and_read_as_handles() {
        let shapes = paint_octave_strip(5, 0, DEFAULT_EXTRA_SIZE, 0.0);
        let hs = handles(&shapes);
        assert_eq!(hs.len(), 2, "the strip did not paint two handles");
        for h in &hs {
            assert!(h.width() >= 4.0, "a handle thinner than this vanishes into the fill");
        }
        // On the outer edge of the wheel, which is what a fringe is dragged
        // out from and the count is dragged in from.
        let plain = cells(&shapes);
        let edges = (plain[0].left(), plain[4].right());
        assert!((hs[0].center().x - edges.0).abs() < 1.0, "the low handle left the edge");
        assert!((hs[1].center().x - edges.1).abs() < 1.0, "and the high one");

        // And with a fringe, where the boundary is INSIDE the wheel rather
        // than on its edge — at zero extras the two coincide, so a strip drawn
        // only there cannot tell the count's boundary from the wheel's.
        let shapes = paint_octave_strip(5, 2, 0.4, 0.0);
        let (hs, fringed) = (handles(&shapes), cells(&shapes));
        assert_eq!(fringed.len(), 9, "five full-size octaves and two extras each end");
        assert!(
            (hs[0].center().x - fringed[2].left()).abs() < 1.0,
            "the low handle is not where the fringe ends and the count starts"
        );
        assert!((hs[1].center().x - fringed[6].right()).abs() < 1.0, "nor the high one");
    }

    /// The widest wheel puts its boundary on the bar's own edge, and a handle
    /// centered there hangs half its width outside — where it reads as the
    /// border rather than as something to grab. That is the bug
    /// `the_handles_read_as_handles_even_at_the_limits` pins for the range
    /// bar, and it arrives here by a different route: not a value at the end
    /// of a scale, but a count that fills every slot of the budget.
    #[test]
    fn the_strips_handles_stay_inside_the_bar_at_the_widest_wheel() {
        let shapes = paint_octave_strip(MAX_SPAN, 0, DEFAULT_EXTRA_SIZE, 0.0);
        let bar = filled_rects(&shapes)[0].0;
        let hs = handles(&shapes);
        assert_eq!(hs.len(), 2, "the widest wheel did not paint two handles");
        for h in &hs {
            assert!(
                h.left() >= bar.left() - 0.01 && h.right() <= bar.right() + 0.01,
                "handle {h:?} hangs outside the bar {bar:?}"
            );
            assert!(h.width() >= 4.0, "a handle thinner than this vanishes into the fill");
        }
    }

    /// Which gesture a press starts is decided by the region it lands in, and
    /// the handles are the border between them. At zero extras both handles
    /// sit on the wheel's outer edge, where a nearest-handle rule would have
    /// nothing to say — and that is exactly the state you first meet.
    #[test]
    fn a_strip_press_takes_the_count_inside_the_handles_and_the_fringe_outside() {
        let inside = |reach: f32, count: u32| {
            matches!(StripGrab::at(reach, count, 0), StripGrab::Count { .. })
        };
        assert!(inside(0.0, 5), "the middle of the wheel is the count");
        assert!(inside(2.4, 5), "just inside the handle is the count");
        assert!(!inside(2.6, 5), "just outside it is the fringe");
        assert!(!inside(4.0, 5), "and so is the empty track past the wheel");
        // The grab remembers the extras it started with, not the ones the
        // budget later leaves — see the round trip below.
        assert!(matches!(StripGrab::at(0.0, 5, 3), StripGrab::Count { extras: 3 }));
    }

    /// Half a slot of travel per octave, in both gestures: the count grows at
    /// both ends of the wheel at once and so does the fringe, so the pointer
    /// is always on the boundary it is dragging.
    #[test]
    fn a_strip_drag_moves_half_a_slot_an_octave() {
        let count = StripGrab::Count { extras: 0 };
        for (reach, want) in [(2.5f32, 5u32), (3.5, 7), (1.5, 3), (5.5, 11)] {
            assert_eq!(count.apply(reach).0, want, "{reach} slots out is not {want} octaves");
        }
        // Measured from the edge of the count, so the fringe reads as octaves
        // added to the wheel rather than as a position on the strip.
        for (reach, want) in [(2.5f32, 0u32), (3.4, 1), (4.5, 2), (5.5, 3)] {
            assert_eq!(
                StripGrab::Extras { count: 5 }.apply(reach).1,
                want,
                "{reach} slots out is not {want} extras past a count of five",
            );
        }
    }

    /// The budget is eleven slices, and the count is what wins inside it: a
    /// drag that raises the count past what the fringe leaves takes the
    /// extras with it, and dragging home again brings them back. The grab
    /// holding the extras the gesture STARTED with is what buys the second
    /// half — re-reading the yielded number every frame would make one drag
    /// out and back a one-way trip.
    #[test]
    fn raising_the_count_yields_the_extras_and_dragging_home_restores_them() {
        let grab = StripGrab::at(0.0, 5, 3);
        assert_eq!(grab.apply(2.5), (5, 3), "the wheel it started on");
        assert_eq!(grab.apply(4.5), (9, 1), "the extras did not yield to the count");
        assert_eq!(grab.apply(5.5), (11, 0), "and the last of them at the ceiling");
        assert_eq!(grab.apply(2.5), (5, 3), "dragging home did not restore the fringe");
        // The fringe cannot overrun the budget either, and a count of one is
        // only drawable with a pair to flank it.
        assert_eq!(
            StripGrab::Extras { count: 5 }.apply(9.0),
            (5, 3),
            "the fringe overran the budget"
        );
        assert_eq!(StripGrab::Count { extras: 0 }.apply(0.0), (MIN_SPAN, 0));
        assert_eq!(StripGrab::Count { extras: 2 }.apply(0.0), (1, 2));
    }

    /// The mirror of the round trip above, and the one the FRINGE gesture
    /// needs for itself. `clamp_wheel` opens a lone full-size octave to two
    /// when the fringe leaves it — its answer for a blob that asks for an
    /// undrawable wheel — so a fringe drag that reads the live count back
    /// moves the count, and then measures every later frame of the same drag
    /// from the number it just moved. Dragging the fringe in off a lone octave
    /// and back out has to land where it started.
    #[test]
    fn a_fringe_drag_off_a_lone_octave_comes_home() {
        // Frame by frame the way `show` runs it: the grab is taken once at the
        // press and every frame after re-applies it.
        let played = |start: (u32, u32), press: f32, frames: &[f32]| {
            let grab = StripGrab::at(press, start.0, start.1);
            assert!(matches!(grab, StripGrab::Extras { .. }), "{press} slots out is the fringe");
            let mut wheel = start;
            for &reach in frames {
                wheel = grab.apply(reach);
            }
            wheel
        };
        // A lone octave with one extra a side — the picture MIN_COUNT exists
        // for — nudged in half a slot and back out to the pixel it started on.
        assert_eq!(
            played((1, 1), 1.4, &[1.4, 0.9, 1.4]),
            (1, 1),
            "a fringe drag out and home moved the count"
        );
        // And the long version, across the whole strip.
        assert_eq!(
            played((1, 5), 5.4, &[5.4, 3.5, 0.9, 3.5, 5.4]),
            (1, 5),
            "the widest fringed wheel did not come home"
        );
    }

    /// Drive real gestures over a 300pt strip: for each `(press, release)`,
    /// press that many slots out from the middle, drag to the second and let
    /// go. Answers the wheel left behind. Through a real `egui::Context` with
    /// real pointer events, which is the only way to reach what the widget
    /// does with egui's own drag threshold — and, across two gestures, with
    /// the grab it remembers between them.
    fn drag_strip(start: (u32, u32), gestures: &[(f32, f32)]) -> (u32, u32) {
        const W: f32 = 300.0;
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(W, 100.0));
        let (mut count, mut extras) = start;
        let track = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut t = 0.0;
        let mut frame = |count: &mut u32, extras: &mut u32, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(t),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let response = OctaveStrip::new(count, extras, 0.4, 0.0).show(ui);
                    track.set(response.rect);
                },
            );
        };
        // A frame with no input first: egui resolves the pointer against the
        // PREVIOUS pass's widget rects, so the strip has to have been laid out
        // once before a press can land on it.
        frame(&mut count, &mut extras, vec![]);
        let bar = track.get();
        let at = |slots: f32| {
            egui::pos2(bar.left() + bar.width() * (0.5 + slots / MAX_SPAN as f32), bar.center().y)
        };
        for &(press, release) in gestures {
            let toward = (release - press).signum();
            frame(&mut count, &mut extras, vec![egui::Event::PointerMoved(at(press))]);
            frame(&mut count, &mut extras, vec![
                egui::Event::PointerMoved(at(press)),
                egui::Event::PointerButton {
                    pos: at(press),
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            // A small step first, then the rest of the way. egui does not call
            // a gesture a drag until the pointer has left a six-point click
            // threshold, so this step is where the widget first sees one — and
            // a step of about half a slot is what a real hand produces at
            // 60fps.
            let step = at(press + 0.44 * toward);
            frame(&mut count, &mut extras, vec![egui::Event::PointerMoved(step)]);
            frame(&mut count, &mut extras, vec![egui::Event::PointerMoved(at(release))]);
            // And let go, which is the only thing that forgets the grab.
            frame(&mut count, &mut extras, vec![egui::Event::PointerButton {
                pos: at(release),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        (count, extras)
    }

    /// The wiring, once: a real press inside the handles dragged outward
    /// changes the COUNT and not the fringe, however far past the handles it
    /// ends up — the grab is taken at the press.
    #[test]
    fn a_real_drag_on_the_strip_keeps_the_gesture_it_started() {
        // A five-octave count reaches 2.5 slots of eleven either side of the
        // middle, so this presses inside the right-hand handle and drags well
        // past it into what is fringe.
        assert_eq!(
            drag_strip((5, 2), &[(2.0, 4.5)]),
            (9, 1),
            "the drag did not carry the count out to the pointer, or the fringe \
             yielded more than the budget demanded"
        );
    }

    /// Letting go forgets which gesture was being held. egui's temp store has
    /// no expiry, so a grab left behind is inherited by the NEXT press — and
    /// since it is read before `at` is consulted, that press never gets to
    /// choose. One stale count grab would make the fringe unreachable for the
    /// rest of the session.
    #[test]
    fn a_second_gesture_on_the_strip_chooses_for_itself() {
        // Drag the count out to nine, let go, then press in the fringe past
        // its new boundary at 4.5 slots and pull outward. Holding the first
        // grab, that second drag would read as a count and land on eleven.
        assert_eq!(
            drag_strip((5, 0), &[(2.0, 4.5), (5.0, 5.4)]),
            (9, 1),
            "the second gesture inherited the first one's grab"
        );
    }

    /// A press within egui's own six-point click threshold of a handle, which
    /// is where half of the handle IS. egui reports no drag until the pointer
    /// has moved that far, so the first frame the widget can decide anything
    /// on is already past the boundary — and this control splits its two
    /// gestures on a hard line, where a `RangeBar` handle has fourteen points
    /// of reach around it. Reading the live pointer there hands the canonical
    /// gesture, grab the handle and pull it out, to the fringe.
    #[test]
    fn a_press_just_inside_a_handle_still_takes_the_count() {
        // 2.4 slots out on a 300pt strip is 2.7pt inside the boundary, and the
        // handle is drawn 2pt either side of it — so this is a press ON the
        // affordance, dragged the way it invites.
        assert_eq!(
            drag_strip((5, 0), &[(2.4, 4.5)]),
            (9, 0),
            "a press on the handle's inner half grew a fringe instead of the count"
        );
        // The mirror: just OUTSIDE the handle, dragging inward, is the fringe.
        assert_eq!(
            drag_strip((5, 2), &[(2.6, 3.6)]),
            (5, 1),
            "a press just outside the handle moved the count"
        );
    }
}
