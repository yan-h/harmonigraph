//! [`LayerStrip`]: which septimal sheets the lattice draws and which of them
//! is home, as one row.

use egui::{CornerRadius, Response, Sense, TextStyle, Ui, Vec2};
use harmonigraph_scene::{ViewConfig, SEVENS_LAYER_LIMIT};

use super::bar::{
    aimed_at, bar_radius, bar_width, elided_name, grabbed, grip_color, grip_over_text, grip_radius,
    grip_rect, poised, release_grab, track_fill, BAR_TEXT_PAD, HANDLE_W,
};
use crate::theme;

/// Gap between two cells, so a stack reads as a row of separate sheets rather
/// than one fill with steps in it. The octave strip's, for the same reason.
const CELL_GAP: f32 = 1.0;

/// Shortest a cell is ever drawn. A sheet dialled to a sixth of the home one
/// comes out under a pixel on a short row, and a cell that is not there says
/// the sheet is not either.
const CELL_MIN_H: f32 = 3.0;

/// Cells on the axis: every sheet [`SEVENS_LAYER_LIMIT`] allows, drawn or not.
const CELLS: i32 = 2 * SEVENS_LAYER_LIMIT + 1;

/// Which of the strip's three handles a drag took hold of, decided on the first
/// frame of the gesture and remembered for the rest of it.
///
/// EVERY variant carries the values it must not move, so [`apply`](Self::apply)
/// is a pure function of where the pointer got to. That is the same rule the
/// octave strip and the range bar are written to, and it is what makes dragging
/// out and home again land where it started: a branch that read a number back
/// mid-gesture would be reading one it had just written.
///
/// `Default` is only egui's, asked of anything its temp store can remove;
/// nothing reads the default, since drag-start always writes first.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    /// The low end. Home comes UP with it when it is dragged past — home names
    /// a sheet the picture draws, so an end that walked over it would leave it
    /// naming one that is not there.
    Low { home: i32, high: i32 },
    /// The high end, the same way round.
    High { low: i32, home: i32 },
    /// Home, between the two ends it cannot leave.
    Home { low: i32, high: i32 },
    /// The whole stack, sliding at a fixed shape: `offset` is how far along it
    /// the pointer took hold, `width` how many steps its ends were apart and
    /// `home` how far home sat from its low end.
    ///
    /// All three are frozen for the gesture, which is what makes sliding into
    /// a wall and back out again stable — the arithmetic never reads the stack
    /// it has itself been pushing.
    Stack { offset: f32, width: i32, home: i32 },
}

impl Default for Grab {
    fn default() -> Self {
        Grab::Stack { offset: 0.0, width: 0, home: 0 }
    }
}

/// Where the strip's handles STAND, in cell units: the two ends on the outer
/// borders of the cells they bound, home in the middle of its own.
///
/// One function because a handle a press is decided against and a handle drawn
/// somewhere else is a control that lights one thumb and moves another. The
/// ends read as brackets around the stack and home as a mark inside it, which
/// is also why the ends are half a cell out rather than on their cell's middle:
/// a bracket on the border is where the span visibly ends.
fn handle_at(low: i32, home: i32, high: i32) -> (f32, f32, f32) {
    (low as f32 - 0.5, home as f32, high as f32 + 0.5)
}

/// The sheet whose cell holds cell position `v`.
///
/// `floor` rather than `round`, which are the same answer everywhere but the
/// exact boundary between two cells and differ there in a way that shows: Rust
/// rounds a half AWAY FROM ZERO, so a press landing precisely on a border would
/// snap up above the middle of the axis and down below it. A handle that jumps
/// a cell on being pressed, in whichever direction it happens to be standing,
/// is the kind of thing a pointer finds more often than arithmetic suggests.
fn cell_at(v: f32) -> i32 {
    (v + 0.5).floor() as i32
}

impl Grab {
    /// What a drag starting at cell position `v` takes hold of.
    ///
    /// By REGION, in cells, with no reach in points anywhere in it — which is
    /// the one thing a strip can do that a bar cannot, and the reason the whole
    /// control is drawn as cells. Every region below is at least half a cell
    /// wide at every stack the axis can hold, so all four gestures survive a
    /// narrow settings column; a proximity rule sized in points does not, and
    /// the way it fails is the interesting part — three handles each claiming
    /// [`GRAB_PX`] either side cover a short stack completely, so the slide
    /// quietly stops existing on exactly the stacks most worth sliding.
    ///
    /// [`GRAB_PX`]: super::bar::GRAB_PX
    ///
    /// The regions, outward from the middle:
    ///
    /// - below the low sheet's middle, the LOW end — its own left half and
    ///   every empty cell beyond it, so the end a stack is pinned at is
    ///   grabbable even standing on the floor of the axis;
    /// - above the high sheet's middle, the HIGH end, the same way round;
    /// - the home cell, HOME;
    /// - what is left, the stack, sliding whole.
    ///
    /// Home wins its own cell against an end that coincides with it, and that
    /// is the case that ships: every sheet above home and none below is an
    /// ordinary stack, and the end there is still reachable from the half-cell
    /// outside it while home would not be reachable at all.
    fn at(v: f32, (low, home, high): (i32, i32, i32)) -> Grab {
        let (e_low, _, e_high) = handle_at(low, home, high);
        // A stack of ONE sheet is the one shape with nothing between its ends,
        // so the rule below would hand its whole cell to an end and leave the
        // sheet unmovable. Its own cell SLIDES instead — which is how the sheet
        // a flat lattice draws is moved, the state a fresh view opens in — and
        // the empty cells either side still open that end.
        if high <= low {
            return if v < e_low {
                Grab::Low { home, high }
            } else if v > e_high {
                Grab::High { low, home }
            } else {
                Grab::Stack { offset: v - e_low, width: high - low, home: home - low }
            };
        }
        if v < low as f32 {
            Grab::Low { home, high }
        } else if v > high as f32 {
            Grab::High { low, home }
        } else if cell_at(v) == home {
            Grab::Home { low, high }
        } else {
            Grab::Stack { offset: v - e_low, width: high - low, home: home - low }
        }
    }

    /// Where the three end up when this grab is dragged to cell position `v`.
    /// Pure, so the invariants that actually matter — the ends never cross,
    /// home never leaves the stack, and a slid stack keeps both its width and
    /// where home sat in it — are testable without a pointer.
    fn apply(self, v: f32) -> (i32, i32, i32) {
        let on_axis = |i: i32| i.clamp(-SEVENS_LAYER_LIMIT, SEVENS_LAYER_LIMIT);
        // The BORDER nearest the pointer, as the index of the sheet ABOVE it —
        // borders run half a cell outside the sheets, so they are numbered one
        // wider than the axis at the top and the clamp belongs on the sheet
        // each arm derives rather than here. Clamping the border instead cost
        // the high end its top sheet: the index it wants at the ceiling is
        // `LIMIT + 1`, so held to `LIMIT` it came out a sheet short, and the
        // last cell of the strip could not be dragged to at all.
        let border = |v: f32| cell_at(v + 0.5);
        match self {
            Grab::Low { home, high } => {
                let low = on_axis(border(v)).min(high);
                (low, home.max(low), high)
            }
            Grab::High { low, home } => {
                // The high end stands on the low border of the cell ABOVE its
                // own sheet, which is the one `border` answers with.
                let high = on_axis(border(v).saturating_sub(1)).max(low);
                (low, home.min(high), high)
            }
            Grab::Home { low, high } => (low, on_axis(cell_at(v)).clamp(low, high), high),
            Grab::Stack { offset, width, home } => {
                // The shape the gesture froze, held to something drawable: a
                // `ViewConfig` assembled field by field can carry a stack wider
                // than the axis or a home outside it, and `clamp` panics on an
                // inverted pair rather than answering one.
                let width = width.clamp(0, 2 * SEVENS_LAYER_LIMIT);
                let home = home.clamp(0, width);
                let low = border(v - offset).clamp(-SEVENS_LAYER_LIMIT, SEVENS_LAYER_LIMIT - width);
                (low, low + home, low + width)
            }
        }
    }

    /// Whether this grab holds the low end, home, and the high end — which is
    /// what [`grip_color`] lights each thumb by. A slide holds all three.
    fn holds(self) -> (bool, bool, bool) {
        match self {
            Grab::Low { .. } => (true, false, false),
            Grab::Home { .. } => (false, true, false),
            Grab::High { .. } => (false, false, true),
            Grab::Stack { .. } => (true, true, true),
        }
    }
}

/// The septimal sheet stack as one control: a strip of every sheet the axis
/// offers, with the drawn ones filled, home the tallest of them, and three
/// handles — the two borders the stack ends on and the mark on home.
///
/// Drag an end to add or drop sheets on that side, drag home to move which
/// sheet the music is heard against, drag between them to slide the whole stack
/// at a fixed shape; double-click goes home to [`reset_stack`].
///
/// **Three handles rather than two bars**, and what that buys is the stack that
/// two bars could not describe. The pair this replaced was a count each side of
/// home and where home sat, so every stack it could ask for was symmetric —
/// there was no way to say *two sheets above home and none below*, and the
/// septimal axis is where that is an ordinary request: the sheets above home
/// and the ones below carry different spellings.
///
/// **A cell per sheet rather than a continuous axis**, which is what makes the
/// ends reachable at all. Three handles on one bar have to be told apart by a
/// hand, and the axis they share is nine steps wide; drawn as cells each
/// one is a target about as wide as a button, and a drag lands on a sheet
/// instead of near one.
///
/// Cell HEIGHT is how big a node on that sheet draws against one on home (see
/// [`ViewConfig::sevens_size`]), so the strip shows the stack's shape as well
/// as its extent — and the tallest cell IS home, which is the reading that
/// makes the middle handle mean something before it is dragged. That is the
/// same argument the octave strip carries its profile on, with the same
/// consequence: two stacks cannot be compared by height, since each is drawn
/// against its own home sheet.
pub struct LayerStrip<'a> {
    low: &'a mut i32,
    home: &'a mut i32,
    high: &'a mut i32,
    /// The size falloff, read-only: it shapes the profile the strip draws and
    /// has its own bar under it.
    size: f32,
}

/// The stack a double-click goes home to: the one a fresh view opens with.
///
/// Read off [`ViewConfig::default`] rather than restated as a literal, for the
/// reason the octave strip's reset is: a reset naming its own numbers goes on
/// resetting to a stack that is merely no longer anyone's default.
pub(super) fn reset_stack() -> (i32, i32, i32) {
    let fresh = ViewConfig::default();
    (fresh.min_sevens, fresh.center_sevens, fresh.max_sevens)
}

impl<'a> LayerStrip<'a> {
    pub fn new(low: &'a mut i32, home: &'a mut i32, high: &'a mut i32, size: f32) -> Self {
        LayerStrip { low, home, high, size }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let scale = theme::ui_scale(ui.ctx());
        let width = bar_width(ui);
        let (rect, mut response) = ui.allocate_exact_size(
            Vec2::new(width, theme::row_height(scale)),
            Sense::click_and_drag(),
        );
        let slot = (rect.width() / CELLS as f32).max(1.0);
        // Cell units and points, the two directions. Cell `i` is centered on
        // `x_of(i)`, so a whole number is a sheet and a half is a border.
        let x_of = |i: f32| rect.left() + (i + SEVENS_LAYER_LIMIT as f32 + 0.5) * slot;
        let at = |x: f32| (x - rect.left()) / slot - SEVENS_LAYER_LIMIT as f32 - 0.5;

        // ---- Interaction ----------------------------------------------------
        let grab_id = response.id.with("grab");
        let mut holding = None;
        if response.double_clicked() {
            (*self.low, *self.home, *self.high) = reset_stack();
            response.mark_changed();
        }
        if response.dragged() {
            if let Some(p) = response.interact_pointer_pos() {
                let stack = (*self.low, *self.home, *self.high);
                let grab = grabbed(ui, grab_id, |ui| {
                    // From where the press LANDED (see `aimed_at`): the first
                    // frame egui calls a drag is already six points along, and
                    // a cell is not much wider than that, so the live position
                    // hands "grab this end and pull it out" to whatever the
                    // hand was aiming past.
                    Grab::at(at(aimed_at(ui, p).x), stack)
                });
                holding = Some(grab);
                let next = grab.apply(at(p.x));
                if next != stack {
                    (*self.low, *self.home, *self.high) = next;
                    response.mark_changed();
                }
            }
        }
        if response.drag_stopped() {
            release_grab::<Grab>(ui, grab_id);
        }

        // ---- Paint ----------------------------------------------------------
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(bar_radius(scale)), theme::well());

        let (low, home, high) = (*self.low, *self.home, *self.high);
        let fill_color = track_fill(&response);
        let gap = CELL_GAP * scale;
        let cell_radius = CornerRadius::same(theme::scaled_points(2, scale));
        // How big a node on sheet `i` draws against one on home. Normalized by
        // the tallest cell DRAWN rather than by home's own, which are the same
        // number for every stack the strip can produce and not for one a blob
        // handed over with home outside its ends.
        let profile = |i: i32| self.size.clamp(0.0, 1.0).powi((i - home).abs());
        let (first, last) = (low.max(-SEVENS_LAYER_LIMIT), high.min(SEVENS_LAYER_LIMIT));
        let tallest = (first..=last).map(profile).fold(0.0f32, f32::max).max(1e-6);
        for i in first..=last {
            let height = (rect.height() * profile(i) / tallest).max(CELL_MIN_H * scale);
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(x_of(i as f32) - 0.5 * slot + 0.5 * gap, rect.bottom() - height),
                    egui::pos2(x_of(i as f32) + 0.5 * slot - 0.5 * gap, rect.bottom()),
                ),
                cell_radius,
                fill_color,
            );
        }

        // Name and readout as a ValueBar wears them, ahead of the handles so a
        // thumb standing on a letter can knock it out.
        let text_color = if response.hovered() || response.dragged() {
            theme::text()
        } else {
            theme::text_dim()
        };
        let mono = TextStyle::Monospace.resolve(ui.style());
        // A stack of one sheet reads out that sheet alone, the way the octave
        // strip drops its fringe from the readout: three copies of one number
        // says there are three things to read when there is one.
        let shown = if low == home && home == high {
            format!("{home}")
        } else {
            format!("{low}·{home}·{high}")
        };
        let value = painter.layout_no_wrap(shown, mono.clone(), theme::text());
        // Room kept clear for the widest readout the axis can produce rather
        // than for the one in it, so the name does not re-elide as a sheet
        // gains a minus sign mid-drag. Derived from the limit, so widening the
        // axis cannot leave the reserve measuring an axis that is gone.
        let widest = format!("{0}·{0}·{0}", -SEVENS_LAYER_LIMIT);
        let reserve = painter.layout_no_wrap(widest, mono, theme::text()).size().x;
        let body = TextStyle::Body.resolve(ui.style());
        let mut job = egui::text::LayoutJob::default();
        job.append("Layers", 0.0, egui::TextFormat::simple(body, text_color));
        let text_pad = BAR_TEXT_PAD * scale;
        let label = elided_name(painter, job, rect.width(), scale, reserve);
        let centered =
            |galley: &egui::Galley, x: f32| egui::pos2(x, rect.center().y - galley.size().y * 0.5);
        let name_at = centered(&label, rect.left() + text_pad);
        painter.galley(name_at, label.clone(), text_color);
        painter.galley(
            centered(&value, rect.right() - text_pad - value.size().x),
            value,
            theme::text(),
        );

        // The three handles, lit by what is in hand (see `grip_color`). The low
        // end reaches the far left of the bar, where the NAME is, so each grip
        // is drawn through `grip_over_text` — the name is pinned and cannot be
        // placed clear of them the way the readout is.
        let inset = 0.5 * HANDLE_W * scale;
        let in_hand =
            holding.or_else(|| poised(ui, &response).map(|p| Grab::at(at(p.x), (low, home, high))));
        let (lit_low, lit_home, lit_high) = in_hand.map_or((true, true, true), Grab::holds);
        let (e_low, c_home, e_high) = handle_at(low, home, high);
        let grips = [(e_low, lit_low), (c_home, lit_home), (e_high, lit_high)];
        let name_run = [(name_at, label)];
        // Unlit first: two thumbs coincide whenever home sits on an end, and
        // the later fill is the colour that shows.
        for pass in [false, true] {
            for (i, _) in grips.iter().filter(|(_, lit)| *lit == pass) {
                // Held inside the bar by half a handle, for the reason
                // `HANDLE_INSET` holds a range bar's ends off theirs: a stack
                // filling the axis puts a border on the bar's own edge, and a
                // handle centered there hangs half its width over the pane,
                // where the part still on the bar reads as its border.
                let x = x_of(*i).clamp(rect.left() + inset, rect.right() - inset);
                grip_over_text(
                    painter,
                    grip_rect(x, rect, scale),
                    grip_radius(scale),
                    grip_color(pass),
                    &name_run,
                );
            }
        }

        response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::probe::{filled_rects, handles, press, shapes, text_boxes};

    /// Paint one strip across a 300pt row and return what it emitted.
    fn paint_strip(low: i32, home: i32, high: i32, size: f32) -> Vec<egui::Shape> {
        let (mut l, mut c, mut h) = (low, home, high);
        shapes(300.0, |ui| {
            LayerStrip::new(&mut l, &mut c, &mut h, size).show(ui);
        })
    }

    /// The strip's cells, left to right: the accent-filled rects, which the
    /// well behind them and the handles over them are not.
    fn cells(shapes: &[egui::Shape]) -> Vec<egui::Rect> {
        let mut cells: Vec<_> = filled_rects(shapes)
            .into_iter()
            .filter(|(_, fill)| *fill == theme::accent_fill())
            .map(|(r, _)| r)
            .collect();
        cells.sort_by(|a, b| a.left().total_cmp(&b.left()));
        cells
    }

    /// One cell per sheet DRAWN, over an axis of every sheet there is — so the
    /// empty track at each end is the depth still unspent, and a cell is the
    /// same width whatever the stack. That is what makes the strip a fixed axis
    /// to drag on rather than one that stretches under the pointer.
    #[test]
    fn the_strip_draws_a_cell_per_sheet_on_a_fixed_axis() {
        let bar = filled_rects(&paint_strip(0, 0, 0, 1.0))[0].0;
        let slot = bar.width() / CELLS as f32;
        for (low, home, high) in [(0, 0, 0), (-1, 0, 1), (0, 0, 2), (-4, -4, 4), (2, 4, 4)] {
            let drawn = cells(&paint_strip(low, home, high, 1.0));
            assert_eq!(
                drawn.len(),
                (high - low + 1) as usize,
                "{low}..{high} drew the wrong number of cells",
            );
            // Where on the axis, not merely how many: a stack leaning one way
            // has to draw there, which a count alone cannot see.
            let left = bar.left() + (low + SEVENS_LAYER_LIMIT) as f32 * slot;
            assert!(
                (drawn[0].left() - left).abs() < 1.5,
                "{low}..{high} starts at {} rather than {left}",
                drawn[0].left(),
            );
        }
    }

    /// Cell height is the node size that sheet draws at, so the tallest cell is
    /// HOME — the one reading that makes the middle handle mean something
    /// before anyone drags it. Asymmetric on purpose: home off center is the
    /// stack the pair this replaced could not ask for.
    #[test]
    fn the_tallest_cell_is_home_wherever_home_sits() {
        for (low, home, high) in [(-2, 0, 2), (0, 0, 3), (-3, 0, 0), (-1, 1, 3)] {
            let drawn = cells(&paint_strip(low, home, high, 0.55));
            let tallest = drawn
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.height().total_cmp(&b.1.height()))
                .expect("a drawn cell")
                .0;
            assert_eq!(
                tallest as i32 + low,
                home,
                "{low}..{high} home {home}: the tallest cell is sheet {}",
                tallest as i32 + low,
            );
        }
        // And the profile is the falloff itself, not merely an ordering: one
        // step off home draws at `sevens_size` of home's own height.
        let drawn = cells(&paint_strip(-1, 0, 1, 0.55));
        let ratio = drawn[0].height() / drawn[1].height();
        assert!((ratio - 0.55).abs() < 0.01, "a sheet one step off home drew {ratio} of home");
    }

    /// Three handles, and each of them wide enough to read as one. The ends
    /// stand on the borders the stack ends at and home in the middle of its own
    /// cell, which is also where a press is decided against them.
    #[test]
    fn the_strip_stands_a_handle_on_each_end_and_on_home() {
        let shapes = paint_strip(-2, 1, 3, 0.55);
        let hs = handles(&shapes);
        assert_eq!(hs.len(), 3, "the strip did not paint three handles");
        for h in &hs {
            assert!(
                (h.width() - HANDLE_W).abs() < 0.01,
                "a strip handle is the grip every bar wears"
            );
        }
        let drawn = cells(&shapes);
        let (first, last) = (drawn[0], drawn[drawn.len() - 1]);
        assert!((hs[0].center().x - first.left()).abs() < 1.5, "the low end is not on its border");
        assert!((hs[2].center().x - last.right()).abs() < 1.5, "the high end is not on its border");
        // Home is sheet 1, the fourth of the five drawn from -2.
        assert!(
            (hs[1].center().x - drawn[3].center().x).abs() < 1.5,
            "the home handle is not on the home cell",
        );
    }

    /// What the strip says in words. Three numbers where every other bar in the
    /// pane carries one, and their ORDER is the whole of what tells a stack from
    /// its mirror — and a stack of one sheet drops the other two rather than
    /// reading out the same number three times.
    #[test]
    fn the_strip_reads_out_the_stack_it_draws() {
        let texts = |shapes: &[egui::Shape]| {
            text_boxes(shapes).into_iter().map(|(_, t)| t).collect::<Vec<_>>()
        };
        assert_eq!(
            texts(&paint_strip(-2, 0, 1, 0.55)),
            vec!["Layers".to_owned(), "-2·0·1".to_owned()],
            "an open stack reads out low, home, high",
        );
        assert_eq!(
            texts(&paint_strip(3, 3, 3, 0.55)),
            vec!["Layers".to_owned(), "3".to_owned()],
            "a single sheet reads out that sheet alone",
        );
    }

    /// A stack of one sheet is where a fresh view opens, and it is the shape
    /// with no interior: all three handles stand within half a cell. A press ON
    /// the cell has to slide it — that is how the sheet a flat lattice draws is
    /// moved — while a press beside it opens that end.
    #[test]
    fn a_single_sheet_slides_from_its_own_cell_and_opens_from_beside_it() {
        assert_eq!(
            Grab::at(0.0, (0, 0, 0)).apply(2.3),
            (2, 2, 2),
            "a press on the lone cell did not slide it",
        );
        assert_eq!(
            Grab::at(-0.8, (0, 0, 0)).apply(-2.3),
            (-2, 0, 0),
            "a press below the lone cell did not open the low end",
        );
        assert_eq!(
            Grab::at(0.8, (0, 0, 0)).apply(2.3),
            (0, 0, 2),
            "a press above the lone cell did not open the high end",
        );
    }

    /// The invariant every gesture owes: `low <= home <= high`. An end dragged
    /// past home carries it rather than stranding it on a sheet the picture no
    /// longer draws, and home dragged at an end stops there.
    #[test]
    fn no_gesture_lets_home_leave_the_stack() {
        // The low end dragged up past home, which is at 0.
        assert_eq!(
            Grab::at(-2.5, (-2, 0, 3)).apply(2.3),
            (3, 3, 3),
            "the low end left home behind it",
        );
        // The high end dragged down past home.
        assert_eq!(
            Grab::at(3.5, (-2, 0, 3)).apply(-0.8),
            (-2, -1, -1),
            "the high end left home behind it",
        );
        // Home dragged past an end stops on it.
        assert_eq!(
            Grab::at(0.0, (-2, 0, 3)).apply(9.0),
            (-2, 3, 3),
            "home walked out past the high end",
        );
        assert_eq!(
            Grab::at(0.0, (-2, 0, 3)).apply(-9.0),
            (-2, -2, 3),
            "home walked out past the low end",
        );
    }

    /// A slid stack keeps its width AND where home sat in it, and squishes
    /// against neither wall — it stops. The gesture reads only its own frozen
    /// shape, so running into the end of the axis and back out again comes home
    /// to where it started rather than to a stack the wall reshaped.
    #[test]
    fn sliding_keeps_the_stacks_shape_through_a_wall_and_back() {
        // Pressed on sheet 1 of a stack that runs -1..2 with home at 0: inside
        // the stack and off the home cell, which is what the slide's region is.
        let grab = Grab::at(1.2, (-1, 0, 2));
        assert!(matches!(grab, Grab::Stack { .. }), "a press in the middle did not take the stack");
        assert_eq!(grab.apply(1.2), (-1, 0, 2), "the press alone moved the stack");
        // One cell short of the wall, deliberately: at `SEVENS_LAYER_LIMIT`
        // this stack's high end reaches the axis after two, so a drag of two
        // would answer the same triple as the slam below and the stop would
        // stop being what is measured.
        assert_eq!(grab.apply(2.2), (0, 1, 3), "the slide did not carry the shape");
        assert_eq!(
            grab.apply(90.0),
            (1, 2, 4),
            "the slide did not stop with its high end on the axis",
        );
        assert_eq!(grab.apply(1.2), (-1, 0, 2), "the slide did not come home");
    }

    /// Every gesture is reachable at EVERY stack the axis can hold, which is
    /// the whole reason the regions are measured in cells rather than in
    /// points. A reach in points is the version that fails here, and quietly:
    /// three handles claiming fourteen points either side cover a short stack
    /// completely on a narrow settings column, so the slide stops existing
    /// exactly where it is most wanted and nothing on screen says so.
    ///
    /// The slide is the one with an honest exception — a stack of one sheet has
    /// no interior, so its own cell IS the slide, which the case below asserts
    /// rather than skips.
    #[test]
    fn every_gesture_has_somewhere_to_be_pressed_at_every_stack() {
        for low in -SEVENS_LAYER_LIMIT..=SEVENS_LAYER_LIMIT {
            for high in low..=SEVENS_LAYER_LIMIT {
                for home in low..=high {
                    // Each cell's middle and its two quarters — a quarter of a
                    // cell apart, so no region half a cell wide falls between
                    // two samples, and every offset is exact in binary so the
                    // sweep cannot drift across a boundary it means to sit on.
                    let taken: Vec<Grab> = (-SEVENS_LAYER_LIMIT..=SEVENS_LAYER_LIMIT)
                        .flat_map(|k| [-0.25, 0.0, 0.25].map(|d| k as f32 + d))
                        .map(|v| Grab::at(v, (low, home, high)))
                        .collect();
                    let has = |want: fn(&Grab) -> bool| taken.iter().any(want);
                    let stack = (low, home, high);
                    // A LONE sheet standing on the end of the axis is the one
                    // exception, and it costs nothing: the end pointing off the
                    // axis has nowhere to go, since outward is off the axis and
                    // inward is past the end it already coincides with. The
                    // other three gestures still answer, which is what moves it.
                    let pinned = |edge: i32| low == high && low == edge;
                    assert!(
                        has(|g| matches!(g, Grab::Low { .. })) || pinned(-SEVENS_LAYER_LIMIT),
                        "{stack:?}: the low end cannot be pressed",
                    );
                    assert!(
                        has(|g| matches!(g, Grab::High { .. })) || pinned(SEVENS_LAYER_LIMIT),
                        "{stack:?}: the high end cannot be pressed",
                    );
                    assert!(
                        has(|g| matches!(g, Grab::Home { .. })) || low == high,
                        "{stack:?}: home cannot be pressed",
                    );
                    assert!(
                        has(|g| matches!(g, Grab::Stack { .. })),
                        "{stack:?}: the stack cannot be slid",
                    );
                }
            }
        }
    }

    /// Each end dragged to the far side of the axis lands ON the far sheet, and
    /// the sweep above cannot see this: it proves each gesture is SELECTABLE
    /// somewhere, never where a maximally dragged handle ends up. The high end
    /// shipped a cell short of the top for exactly that reason — its border is
    /// numbered a sheet wider than the axis, so a clamp on the border rather
    /// than on the sheet left the last cell of the strip undraggable, with a
    /// dead run of bar the pointer could enter and nothing happen.
    #[test]
    fn either_end_dragged_to_the_edge_lands_on_the_last_sheet() {
        const LIMIT: i32 = SEVENS_LAYER_LIMIT;
        // Off the end of the axis, so the clamp rather than the pointer is what
        // decides where each stops.
        let (low_end, high_end) = (-LIMIT as f32 - 3.0, LIMIT as f32 + 3.0);
        assert_eq!(
            Grab::at(high_end, (-2, 0, 3)).apply(high_end),
            (-2, 0, LIMIT),
            "the high end stopped short of the top sheet",
        );
        assert_eq!(
            Grab::at(low_end, (-2, 0, 3)).apply(low_end),
            (-LIMIT, 0, 3),
            "the low end stopped short of the bottom sheet",
        );
        // And the whole axis is reachable as one stack, from either direction.
        assert_eq!(Grab::at(low_end, (-2, 0, 3)).apply(low_end).0, -LIMIT);
        assert_eq!(Grab::at(high_end, (-LIMIT, 0, 3)).apply(high_end), (-LIMIT, 0, LIMIT));
    }

    /// Double-clicking is the only way back to the stock stack, so where it
    /// lands has to BE the stock stack — not a triple that was the stock stack
    /// when the gesture was written.
    #[test]
    fn a_double_click_goes_home_to_the_stack_a_fresh_view_opens_with() {
        let fresh = ViewConfig::default();
        assert_eq!(reset_stack(), (fresh.min_sevens, fresh.center_sevens, fresh.max_sevens));
        assert_eq!(reset_stack(), (0, 0, 0), "a fresh view opens on the home sheet alone");
    }

    /// A press decides against the handle it was AIMED at and the thumb lights
    /// to match, which is the one thing a `ResizeHorizontal` cursor cannot say
    /// on a bar with three of them. Driven through real pointer events, since
    /// the lighting is what the pure arithmetic above cannot see.
    #[test]
    fn hovering_an_end_lights_that_end_alone() {
        use crate::widgets::probe::{after_passes, grips};
        let (mut l, mut c, mut h) = (-2, 0, 2);
        let shapes = after_passes(
            300.0,
            |bar| {
                // Over the low end's border, which is two and a half cells left
                // of the middle of a nine-cell axis.
                let slot = bar.width() / CELLS as f32;
                vec![vec![egui::Event::PointerMoved(egui::pos2(
                    bar.center().x - 2.5 * slot,
                    bar.center().y,
                ))]]
            },
            |ui| LayerStrip::new(&mut l, &mut c, &mut h, 0.55).show(ui),
        );
        let lit: Vec<bool> = grips(&shapes).into_iter().map(|(_, lit)| lit).collect();
        assert_eq!(lit.iter().filter(|l| **l).count(), 1, "hovering an end lit {lit:?}");
    }

    /// A gesture is decided ONCE and holds, so an end dragged clean past its
    /// partner does not become the partner half way. Through real events,
    /// because the grab store is what the rule lives in.
    #[test]
    fn a_drag_keeps_the_handle_it_started_on() {
        let (mut l, mut c, mut h) = (-3, 0, 3);
        let _ = after_passes_on(&mut l, &mut c, &mut h);
        assert_eq!((l, c, h), (3, 3, 3), "the low end did not carry the stack closed");
    }

    /// The drag in the test above: press on the low end's border and pull it
    /// all the way to the far end of the axis in two frames.
    fn after_passes_on(l: &mut i32, c: &mut i32, h: &mut i32) -> Vec<egui::Shape> {
        use crate::widgets::probe::after_passes;
        after_passes(
            300.0,
            |bar| {
                let slot = bar.width() / CELLS as f32;
                let y = bar.center().y;
                let from = egui::pos2(bar.center().x - 3.5 * slot, y);
                vec![
                    vec![egui::Event::PointerMoved(from), press(from, true)],
                    vec![egui::Event::PointerMoved(egui::pos2(bar.center().x + 2.0 * slot, y))],
                    vec![egui::Event::PointerMoved(egui::pos2(bar.center().x + 5.0 * slot, y))],
                ]
            },
            |ui| LayerStrip::new(l, c, h, 0.55).show(ui),
        )
    }
}
