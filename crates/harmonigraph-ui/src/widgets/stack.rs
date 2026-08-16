//! [`StackBar`]: the four layers of a node in one bar — where each one ends,
//! the gaps standing between them, and a handle apiece to size them by.

use egui::{CornerRadius, Response, Sense, TextStyle, Ui, Vec2};
use harmonigraph_scene::{
    RingStack, ViewConfig, CORE_RADIUS_MAX, MARK_THICKNESS_MAX, QUAD_MARGIN, RING_WIDTH_MAX,
};

use super::bar::{
    aimed_at, bar_radius, bar_width, elided_name, grabbed, grip_over_text, release_grab,
    track_fill, BAR_TEXT_PAD, HANDLE_INSET, HANDLE_W,
};
use crate::theme;

/// Closest two of the four thumbs are ever DRAWN, and so the least bar any one
/// of them can be pressed on.
///
/// A layer switched off takes no room, so its boundary stands on the boundary
/// inside it and a bar that placed both honestly would have two thumbs on one
/// point — three or four of them on a node dialled most of the way down. Which
/// of those a press meant is then not a question a position can answer, and the
/// layer that most needs answering is exactly the one with nothing to grab: an
/// off layer is switched back on from its own handle or not at all.
///
/// So the thumbs are pushed apart to this and the presses split on where they
/// are pushed to, which is [`RangeBar`]'s own bargain — drawing a handle where
/// it can be operated rather than where its value is — at the one place this
/// bar needs it. A hair over a thumb's own width, so two of them stand shoulder
/// to shoulder with a seam rather than merged into one wide grip.
///
/// [`RangeBar`]: super::range::RangeBar
const THUMB_SEP: f32 = HANDLE_W + 1.0;

/// Which layer's width a drag is moving.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Layer {
    #[default]
    Core,
    Audio,
    Band,
    Mark,
}

/// What a drag on this bar has hold of: a layer, and how far from that layer's
/// boundary the press landed.
///
/// Decided on the first frame of the gesture and remembered for the rest of it.
/// The LAYER has to be, since the boundaries slide as the layers inside them
/// change and a nearest-thumb rule would hand the gesture on mid-drag.
///
/// **The offset is what makes this a resize rather than a set**, and it is the
/// one place this bar parts company with [`ValueBar`]'s drag-anywhere-to-set.
/// Every stretch of this bar means a layer, so a press lands inside a cell as
/// readily as on its edge — and a boundary that jumped to the pointer would
/// take a press in the middle of the octave band as an instruction to halve it.
/// Held at the press's own distance instead, the band widens by exactly what
/// the hand travelled, from wherever the hand took hold.
///
/// Frozen for the gesture, so the drag reads only where the pointer started and
/// where it is now — never a boundary it moved itself. That is what lets a drag
/// against a wall spring back on the way home instead of creeping, and it is
/// the discipline a [`RangeBar`] span keeps for the same reason.
///
/// (`Default` is derived only to satisfy egui's `remove_temp` bound; the value
/// is always written by drag-start before anything reads it.)
///
/// [`ValueBar`]: super::value::ValueBar
/// [`RangeBar`]: super::range::RangeBar
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Grab {
    layer: Layer,
    offset: f32,
}

/// The stack, innermost first — the order every array here is in, and the
/// order the bar draws.
const LAYERS: [Layer; 4] = [Layer::Core, Layer::Audio, Layer::Band, Layer::Mark];

impl Layer {
    fn index(self) -> usize {
        match self {
            Layer::Core => 0,
            Layer::Audio => 1,
            Layer::Band => 2,
            Layer::Mark => 3,
        }
    }

    /// The field this layer's width is stored in. One accessor rather than four
    /// call sites naming a field, so a drag, a readout and a reset cannot come
    /// to disagree about which number a handle owns.
    fn width(self, view: &ViewConfig) -> f32 {
        match self {
            Layer::Core => view.core_radius,
            Layer::Audio => view.spectral_ring_width,
            Layer::Band => view.band_width,
            Layer::Mark => view.mark_thickness,
        }
    }

    fn set(self, view: &mut ViewConfig, width: f32) {
        *match self {
            Layer::Core => &mut view.core_radius,
            Layer::Audio => &mut view.spectral_ring_width,
            Layer::Band => &mut view.band_width,
            Layer::Mark => &mut view.mark_thickness,
        } = width;
    }
}

/// Where a drag on layer `k` leaves that layer's width, to put its boundary at
/// `to` on the axis with the stack's own boundaries at `edges`.
///
/// Pure, so what actually matters — the boundary landing where the gesture asks
/// for it, the off position at the bottom of every layer's travel, and a ring
/// stopping at the quad edge — is testable without a pointer.
///
/// **Every one of the four reads only the stack INSIDE it**: the core reads
/// nothing, a ring its own slot's start, the marks theirs. So a gesture can
/// never re-read a number it moved itself, and the boundary it is dragging
/// stays a fixed distance from the width it is writing however far the drag
/// runs. A formula that read the boundary OUTSIDE the layer would re-measure a
/// stack the drag had already slid and creep while the pointer sat still.
///
/// The `pad` is what makes the arithmetic uniform across layers that are not
/// uniform. A boundary is one [`gap`](RingStack::gap) past the layer it closes,
/// because the next layer out stands off it — except the marks, which close the
/// stack and stand off nobody, so their boundary is flush with the strip's own
/// end.
///
/// **Under a gap of travel every layer reads 0 and switches off**, which is the
/// price of the stack closing up around a layer that is not there: a width of
/// nothing gives back its slot AND its padding, so the boundary jumps a gap
/// inward the moment the layer goes. The bar has that gap of dead travel at the
/// bottom of every handle rather than a step it could be dragged across, and
/// the alternative — sliding the boundary over ground the picture will not draw
/// it on — is the bar lying about where the layer inside it ends.
fn resized(k: usize, to: f32, edges: [f32; 4], gap: f32) -> f32 {
    let (start, pad) = match k {
        0 => (0.0, gap),
        1 => (edges[0], gap),
        2 => (edges[1], gap),
        _ => (edges[2], 0.0),
    };
    // A ring's own drag stops at the quad edge, so a handle dragged to the end
    // of the axis leaves the widest layer that still FITS rather than one the
    // stack would refuse — which would drop the ring off the node at the top of
    // its own travel, and take every layer outside it along. Refusal is still
    // reachable, from the inside: widen the core past what the ring needs and
    // the ring goes, which is the stack running out of room rather than a bar
    // asking for a size that never made sense.
    //
    // The marks are the one layer with no wall to stop at. They are drawn into
    // the billboard's margin past the quad, which is what the axis's last
    // stretch is (see [`QUAD_MARGIN`]).
    //
    // `min`/`max` rather than a clamp against a computed floor: a stack whose
    // rings are already past the quad hands in a `start` above 1, and
    // `f32::clamp` asserts `min <= max` and takes the editor down with it.
    let high = match k {
        0 => CORE_RADIUS_MAX,
        3 => MARK_THICKNESS_MAX,
        _ => RING_WIDTH_MAX.min((1.0 - start).max(0.0)),
    };
    (to - pad - start).clamp(0.0, high)
}

/// Where the four thumbs are DRAWN, and so where the presses that take them
/// split: each boundary's own place on the bar, pushed out so that no two stand
/// closer than `sep` and the innermost stands clear of the bar's end.
///
/// Pushed OUTWARD, so the innermost thumb of a pile is the one telling the
/// truth and the drift is spent on the layers that have collapsed onto it —
/// which are, by construction, the ones with no width for the drift to
/// misreport. A pile of all four is the widest lie this can tell, three
/// separations of a thumb's width apiece, and it is the state a node with one
/// layer left is genuinely in.
///
/// The innermost is pushed clear of the bar's own end as well, so a layer sized
/// to nothing at the bottom of the axis is still something to take hold of.
/// That is the widest the drift ever is at rest — a separation out of the
/// track, under two hundredths of the axis — and what it buys is the gesture
/// that turns the core back on.
///
/// Then pulled back in from the far end, so a stack dragged past the top of the
/// axis keeps its thumbs on the bar rather than stacking them under the corner
/// where none can be grabbed. On a row too narrow to seat four thumbs at all
/// the second pass wins and the innermost run off the near end — well under the
/// width the panes are held to, and no worse than a bar with no handles.
fn spread(xs: [f32; 4], (left, right): (f32, f32), sep: f32) -> [f32; 4] {
    let mut out = xs;
    let mut floor = left + sep;
    for x in &mut out {
        *x = x.max(floor);
        floor = *x + sep;
    }
    let mut ceil = right - sep;
    for x in out.iter_mut().rev() {
        *x = x.min(ceil);
        ceil = *x - sep;
    }
    out
}

/// Which layer a press at `x` takes hold of: the innermost thumb standing at or
/// past it, and the outermost for a press past them all.
///
/// By REGION and not by the nearer thumb, which is what gives every layer a
/// stretch of bar that means it — a layer's own body, and the gap it stands off
/// by — so pressing on a ring and dragging is that ring widening. A
/// nearest-thumb rule would hand the inner half of every layer to the boundary
/// inside it, and a press on the middle of the audio ring would move the core.
///
/// The empty track past the outermost thumb belongs to the marks, which is the
/// layer a press out there is aiming at: it is the only one whose room is out
/// there.
fn aimed(x: f32, thumbs: [f32; 4], half_thumb: f32) -> Layer {
    LAYERS
        .iter()
        .zip(thumbs)
        .find(|(_, thumb)| x <= thumb + half_thumb)
        .map_or(Layer::Mark, |(layer, _)| *layer)
}

/// The four sizes of a node's layer stack in one bar, drawn as the node's own
/// cross-section: the core out from the center, the audio ring, the octave
/// band and the melody/bass strip, each a cell as long as it is thick, with the
/// Gap standing between them.
///
/// **One control rather than four, because the four were never independent
/// numbers.** Each layer's inner edge is a sum over every layer inside it, so
/// four bars asking for four widths could only be read one at a time and none
/// of them said where its layer actually landed — the one question a size on a
/// node is asked. Here the answer is the picture, and the drag is the same
/// gesture the reading is: a handle is a layer's outer edge, and pulling it out
/// is that layer getting thicker.
///
/// **Four handles for four layers**, each standing at the boundary its layer
/// ends on — the audio ring's inner radius closes the core, the band's closes
/// the ring, the strip's closes the band, and the strip's own outer edge closes
/// the strip. So each handle sizes the layer INSIDE it and slides everything
/// outside it along, which is exactly what the stack does on screen
/// ([`ViewConfig::rings`]).
///
/// **The axis is the node's whole reach, not the quad**: it runs to
/// [`QUAD_MARGIN`], where the mark strip has been eased to nothing and past
/// which a node draws nothing at all. The quad edge sits inside it, marked by a
/// hairline, and it is a real wall rather than a scale mark — a RING that no
/// longer fits inside it is refused rather than clipped, so the layers drop off
/// the outside of the stack one at a time as the room runs out. The room past
/// it is the billboard's margin, which the marks alone are allowed into.
///
/// **0 is every layer's off position**, reached by dragging its handle back
/// onto the one inside it, and the gaps go with it: a layer at 0 gives up its
/// slot and its padding together, so the bar shows the stack closing up rather
/// than a hole where the layer was.
///
/// Double-click restores the four sizes a fresh view opens with. It is not
/// [`ValueBar`]'s type-a-value gesture, for [`RangeBar`]'s reason doubled: a
/// bar with four values has no single one to type into it.
///
/// [`ValueBar`]: super::value::ValueBar
/// [`RangeBar`]: super::range::RangeBar
pub struct StackBar<'a> {
    view: &'a mut ViewConfig,
    label: &'a str,
}

impl<'a> StackBar<'a> {
    pub fn new(view: &'a mut ViewConfig, label: &'a str) -> Self {
        StackBar { view, label }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let scale = theme::ui_scale(ui.ctx());
        let width = bar_width(ui);
        let (rect, mut response) = ui.allocate_exact_size(
            Vec2::new(width, theme::row_height(scale)),
            Sense::click_and_drag(),
        );
        // The axis is inset the way every two-handle bar's is, so a boundary at
        // either limit still seats a whole thumb inside the bar. It matters more
        // here than on a range: a node with nothing but a core parks three
        // thumbs at the bottom of the axis, and a fresh view already stands the
        // outermost at 0.98 of the quad.
        let inset = HANDLE_INSET * scale;
        let track = rect.shrink2(Vec2::new(inset, 0.0));
        let x_of = |v: f32| track.left() + track.width() * (v / QUAD_MARGIN).clamp(0.0, 1.0);
        let value_at = |x: f32| {
            ((x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0) * QUAD_MARGIN
        };
        let sep = THUMB_SEP * scale;
        let half_thumb = HANDLE_W * 0.5 * scale;
        let thumbs_of = |rings: &RingStack| {
            spread(rings.edges().map(&x_of), (track.left(), track.right()), sep)
        };

        // ---- Interaction ----------------------------------------------------
        let grab_id = response.id.with("grab");
        if response.double_clicked() {
            let fresh = ViewConfig::default();
            for layer in LAYERS {
                layer.set(self.view, layer.width(&fresh));
            }
            response.mark_changed();
        }
        if response.dragged() {
            if let Some(p) = response.interact_pointer_pos() {
                let rings = self.view.rings();
                // Decided from where the press LANDED (see `aimed_at`) rather
                // than from where the pointer has since got to: the first frame
                // of a drag arrives a click threshold along, which on a bar
                // whose thumbs can stand a thumb's width apart is enough to
                // point past two of them.
                let grab = grabbed(ui, grab_id, |ui| {
                    let aim = aimed_at(ui, p).x;
                    let layer = aimed(aim, thumbs_of(&rings), half_thumb);
                    // Off the boundary's TRUE place rather than the thumb's
                    // drawn one, so a press on a thumb that was pushed out of a
                    // pile keeps that push for the gesture instead of jumping
                    // the layer out by it.
                    Grab { layer, offset: value_at(aim) - rings.edges()[layer.index()] }
                });
                let to = value_at(p.x) - grab.offset;
                let want = resized(grab.layer.index(), to, rings.edges(), rings.gap);
                if want != grab.layer.width(self.view) {
                    grab.layer.set(self.view, want);
                    response.mark_changed();
                }
            }
        }
        if response.drag_stopped() {
            release_grab::<Grab>(ui, grab_id);
        }

        // ---- Paint ----------------------------------------------------------
        let rings = self.view.rings();
        // What the stack DREW, which is what the cells are: a ring is the width
        // its handle reads or it is not on the node at all, so an empty pair is
        // the one test for "this layer is here" and there is no second flag to
        // disagree with it.
        let spans = [
            (0.0, rings.core_radius),
            rings.audio,
            rings.band,
            (rings.mark_inner, rings.mark_inner + rings.mark_thickness),
        ];
        let r = bar_radius(scale);
        let radius = CornerRadius::same(r);
        let painter = ui.painter();
        painter.rect_filled(rect, radius, theme::well());

        // The gaps first, under the cells that stand them off. A run of faint
        // surface rather than bare track, so the padding the Gap bar sets reads
        // as a part of the node — laid down between two layers — where the
        // empty run past the outermost layer is room the node has not taken.
        // Bare track for both would say a node with its band off and one with
        // its band pushed out are the same picture.
        let mut prev = 0.0f32;
        for (lo, hi) in spans {
            if hi > lo {
                if lo > prev {
                    painter.rect_filled(
                        egui::Rect::from_x_y_ranges(x_of(prev)..=x_of(lo), rect.y_range()),
                        CornerRadius::ZERO,
                        theme::surface_faint(),
                    );
                }
                prev = hi;
            }
        }

        let fill = track_fill(&response);
        // The strip drawn in the plain widget fill when neither end is marked:
        // its depth is still this bar's to set, and the node still keeps the
        // room, but nothing is wearing it. The alternative is greying the whole
        // bar on a checkbox two sections down, which would take the other three
        // layers with it.
        let marked = self.view.mark_melody || self.view.mark_bass;
        for (i, (lo, hi)) in spans.into_iter().enumerate() {
            if hi > lo {
                painter.rect_filled(
                    egui::Rect::from_x_y_ranges(x_of(lo)..=x_of(hi), rect.y_range()),
                    radius,
                    if i == 3 && !marked { theme::widget() } else { fill },
                );
            }
        }

        // The quad edge, where a ring stops fitting and the margin the marks
        // run out into begins. A hairline and not a cell: nothing is drawn
        // there, it is where the drawing stops being possible.
        painter.vline(
            x_of(1.0),
            rect.y_range(),
            egui::Stroke::new(1.0, theme::hairline()),
        );

        // The name where every other bar puts its own, and the four widths
        // parked at the far end in monospace, in stack order — digits that line
        // up and do not wiggle as they change.
        //
        // The sizes DRAWN, which on three of the four is the handle's own value
        // and on a layer the stack had no room for is 0: the bar is a picture of
        // the node, and a cell that is not on screen reading out a width is the
        // disagreement the whole control exists to close. The value is not lost
        // — pull the core back in and the ring returns at the width it kept.
        let text_color = if response.hovered() || response.dragged() {
            theme::text()
        } else {
            theme::text_dim()
        };
        let mono = TextStyle::Monospace.resolve(ui.style());
        let sizes = spans
            .iter()
            .map(|(lo, hi)| format!("{:.2}", hi - lo))
            .collect::<Vec<_>>()
            .join(" ");
        let readout = painter.layout_no_wrap(sizes, mono, theme::text());
        let text_pad = BAR_TEXT_PAD * scale;
        // Four numbers is a long run for one row, and a column can be dragged
        // narrower than it: parked at the far end, held off that end by the same
        // inset the name keeps from the near one, and STOOD DOWN where that
        // would start it inside the near inset — a run that cannot sit on the
        // bar would otherwise be drawn off the pane, where it can be neither
        // read nor scrolled to.
        //
        // Standing it down rather than eliding it, because a number is not a
        // name: half of "0.19" is a different width, where half of "Layers" is
        // still the word. And rather than shortening it to the layer under the
        // pointer, which would leave the bar's one readout saying a different
        // thing on a narrow column than on a wide one. The cells are the reading
        // either way; the numbers are what a wide column can afford.
        let readout_left = rect.right() - text_pad - readout.size().x;
        let numbered = readout_left >= rect.left() + text_pad;
        let body = TextStyle::Body.resolve(ui.style());
        let job = egui::text::LayoutJob::simple_singleline(self.label.to_owned(), body, text_color);
        // The name takes the whole row back when the numbers stand down, so a
        // narrow column reads "Layers" rather than an ellipsis holding room for
        // a run that is not drawn.
        let reserve = if numbered { readout.size().x } else { 0.0 };
        let label = elided_name(painter, job, rect.width(), scale, reserve);
        let centered =
            |galley: &egui::Galley, x: f32| egui::pos2(x, rect.center().y - galley.size().y * 0.5);
        let label_pos = centered(&label, rect.left() + text_pad);
        let readout_pos = centered(&readout, readout_left);
        painter.galley(label_pos, label.clone(), text_color);
        if numbered {
            painter.galley(readout_pos, readout.clone(), theme::text());
        }

        // The thumbs last, over the text: they are the part you operate, and a
        // digit sliding under one is a better outcome than a handle
        // disappearing behind a digit. Both runs are knocked back out through
        // the grip, since neither can be placed clear of four thumbs roaming
        // one row — the name is pinned to the left and the readout parked at
        // the right, and a stack dialled small piles every thumb on the name
        // while a fresh one stands the outermost in the numbers.
        let grip_radius = CornerRadius::same(theme::scaled_points(2, scale));
        let mut runs = vec![(label_pos, label.clone())];
        if numbered {
            runs.push((readout_pos, readout.clone()));
        }
        for x in thumbs_of(&rings) {
            grip_over_text(
                painter,
                egui::Rect::from_center_size(
                    egui::pos2(x, rect.center().y),
                    Vec2::new(HANDLE_W * scale, rect.height() - 3.0 * scale),
                ),
                grip_radius,
                &runs,
            );
        }

        response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::probe::{filled_rects, handles, press, shapes};

    /// A fresh view, which is the stack every claim here is measured against
    /// unless it says otherwise.
    fn fresh() -> ViewConfig {
        ViewConfig::default()
    }

    /// `view` with layer `k`'s handle dragged to `v` up the axis.
    fn dragged(view: &ViewConfig, k: usize, v: f32) -> ViewConfig {
        let mut out = view.clone();
        let rings = out.rings();
        let width = resized(k, v, rings.edges(), rings.gap);
        LAYERS[k].set(&mut out, width);
        out
    }

    /// A handle dragged to a point on the axis puts its layer's boundary
    /// exactly there — which is the whole promise of the control, and the one
    /// that four width bars could not make.
    #[test]
    fn a_handle_lands_where_it_is_dragged() {
        let view = fresh();
        // Each inside what the wall past it allows: a ring's boundary is its
        // own outer edge plus a gap, so the band's cannot pass the quad edge by
        // more than that.
        for (k, target) in [(0, 0.5), (1, 0.9), (2, 1.0), (3, 1.1)] {
            let moved = dragged(&view, k, target);
            assert!(
                (moved.rings().edges()[k] - target).abs() < 1e-5,
                "layer {k} dragged to {target} left its boundary at {}",
                moved.rings().edges()[k],
            );
        }
    }

    /// And the layers outside it slide along keeping their widths, rather than
    /// being eaten by the one being dragged.
    #[test]
    fn the_layers_outside_a_dragged_one_keep_their_widths() {
        let view = fresh();
        // As far out as the core can go with every layer outside it still
        // fitting; past that the stack starts refusing them, which is the next
        // test.
        let moved = dragged(&view, 0, 0.4);
        for layer in &LAYERS[1..] {
            assert_eq!(
                layer.width(&moved),
                layer.width(&view),
                "{layer:?} lost width to the core being dragged",
            );
        }
        let (before, after) = (view.rings().edges(), moved.rings().edges());
        for k in 1..4 {
            assert!(
                (after[k] - before[k] - (after[0] - before[0])).abs() < 1e-5,
                "layer {k}'s boundary did not slide with the core's",
            );
        }
    }

    /// The bottom of every handle's travel is that layer's off position, and it
    /// takes the layer's gap with it: the boundary lands back on the one inside
    /// it rather than a padding out from it.
    #[test]
    fn dragging_a_handle_home_switches_its_layer_off() {
        let view = fresh();
        for (k, layer) in LAYERS.iter().enumerate() {
            let inside = if k == 0 { 0.0 } else { view.rings().edges()[k - 1] };
            let off = dragged(&view, k, inside);
            assert_eq!(layer.width(&off), 0.0, "{layer:?} did not switch off at its floor");
            assert_eq!(
                off.rings().edges()[k],
                inside,
                "layer {k} switched off but kept a gap of room",
            );
        }
    }

    /// A ring dragged to the end of the axis is left the widest one that still
    /// FITS: the quad edge is a wall a handle stops at rather than one it drops
    /// its own layer over.
    #[test]
    fn a_ring_dragged_past_the_quad_edge_stops_at_it() {
        let view = fresh();
        for k in [1, 2] {
            let out = dragged(&view, k, QUAD_MARGIN);
            let (lo, hi) = if k == 1 { out.rings().audio } else { out.rings().band };
            assert!(hi > lo, "layer {k} dragged to the end of the axis came off the node");
            assert!(hi <= 1.0 + 1e-6, "layer {k} was left reaching {hi}, past the quad edge");
        }
    }

    /// Widening a layer past what the stack has room for drops the ones outside
    /// it instead of thinning them — the wall a handle stops at is its own, and
    /// the room running out from the inside is a different thing.
    #[test]
    fn a_layer_widened_past_the_room_left_drops_the_ones_outside_it() {
        let view = fresh();
        // Far enough out that the band no longer fits, and not so far that the
        // audio ring inside it goes too — the stack drops from the OUTSIDE in,
        // one layer at a time.
        let far = dragged(&view, 0, 0.6);
        let rings = far.rings();
        assert!(rings.band.1 <= rings.band.0, "the band was kept at a width it had no room for");
        assert_eq!(far.band_width, view.band_width, "the band's own size was written by the core");
        assert!(rings.audio.1 > rings.audio.0, "the audio ring went with it, from further in");
    }

    /// The marks are the one layer with no such wall — the axis's last stretch
    /// is the billboard margin they are drawn into.
    #[test]
    fn the_marks_are_allowed_past_the_quad_edge() {
        let view = fresh();
        let out = dragged(&view, 3, QUAD_MARGIN);
        let rings = out.rings();
        assert!(
            rings.mark_inner + rings.mark_thickness > 1.0,
            "the strip was held inside the quad at {}",
            rings.mark_inner + rings.mark_thickness,
        );
        assert!(out.mark_thickness <= MARK_THICKNESS_MAX);
    }

    /// Every one of the four thumbs can be pressed, on a node dialled down to
    /// nothing but its core — the state where all four boundaries stand on one
    /// point, and the state a bar that placed them honestly could not be
    /// dragged out of.
    #[test]
    fn a_pile_of_thumbs_still_answers_four_presses() {
        let mut view = fresh();
        for layer in &LAYERS[1..] {
            layer.set(&mut view, 0.0);
        }
        let edges = view.rings().edges();
        assert_eq!(edges[0], edges[3], "the four boundaries were meant to be piled here");
        let thumbs = spread([100.0; 4], (0.0, 400.0), THUMB_SEP);
        let taken: Vec<Layer> =
            thumbs.iter().map(|&x| aimed(x, thumbs, HANDLE_W * 0.5)).collect();
        assert_eq!(taken, LAYERS.to_vec(), "a press on each thumb did not take each layer");
    }

    /// A press inside a layer's own stretch of bar takes that layer, not the
    /// boundary nearest it — pressing on the middle of the audio ring and
    /// dragging is the ring widening.
    #[test]
    fn a_press_on_a_layer_takes_that_layer() {
        let thumbs = [40.0, 120.0, 200.0, 260.0];
        for (x, want) in [
            (10.0, Layer::Core),
            (45.0, Layer::Audio),
            (118.0, Layer::Audio),
            (150.0, Layer::Band),
            (230.0, Layer::Mark),
            (390.0, Layer::Mark),
        ] {
            assert_eq!(aimed(x, thumbs, HANDLE_W * 0.5), want, "a press at {x} took the wrong layer");
        }
    }

    /// The thumbs are pushed apart only where they would otherwise collide, and
    /// the innermost of a pile keeps its own place.
    #[test]
    fn spreading_moves_only_the_thumbs_that_would_overlap() {
        let apart = [40.0, 120.0, 200.0, 260.0];
        assert_eq!(spread(apart, (0.0, 400.0), THUMB_SEP), apart);
        let piled = spread([100.0; 4], (0.0, 400.0), THUMB_SEP);
        assert_eq!(piled[0], 100.0, "the innermost thumb of a pile moved");
        for pair in piled.windows(2) {
            assert!(
                pair[1] - pair[0] >= THUMB_SEP - 1e-5,
                "two thumbs came out {} apart",
                pair[1] - pair[0],
            );
        }
    }

    /// A stack standing past the top of the axis keeps its thumbs on the bar,
    /// where they can still be grabbed.
    #[test]
    fn a_stack_past_the_end_keeps_its_thumbs_on_the_bar() {
        let out = spread([400.0; 4], (0.0, 400.0), THUMB_SEP);
        assert!(out[3] <= 400.0 - THUMB_SEP + 1e-5, "the outermost thumb sat off the bar");
        for pair in out.windows(2) {
            assert!(pair[1] - pair[0] >= THUMB_SEP - 1e-5);
        }
    }

    /// The bar paints one cell per layer that is on the node, and the gaps
    /// between them, at the radii the stack put them at.
    #[test]
    fn the_cells_are_the_stack_the_node_draws() {
        let mut view = fresh();
        let shapes = shapes(W, |ui| {
            StackBar::new(&mut view, "Layers").show(ui);
        });
        let rings = fresh().rings();
        let fills = filled_rects(&shapes);
        let track = fills.first().expect("the bar drew no track").0;
        let inner = track.shrink2(egui::vec2(HANDLE_INSET, 0.0));
        let x_of = |v: f32| inner.left() + inner.width() * v / QUAD_MARGIN;
        for (lo, hi) in [(0.0, rings.core_radius), rings.audio, rings.band] {
            assert!(
                fills.iter().any(|(r, _)| (r.left() - x_of(lo)).abs() < 0.5
                    && (r.right() - x_of(hi)).abs() < 0.5),
                "no cell was drawn for the layer at {lo}..{hi}",
            );
        }
        let gaps = fills.iter().filter(|(_, c)| *c == theme::surface_faint()).count();
        assert_eq!(gaps, 3, "a fresh node stands three gaps between its four layers");
    }

    /// And one thumb per layer, four of them, on the boundaries.
    #[test]
    fn the_bar_draws_a_thumb_for_every_layer() {
        let mut view = fresh();
        let shapes = shapes(W, |ui| {
            StackBar::new(&mut view, "Layers").show(ui);
        });
        assert_eq!(handles(&shapes).len(), 4);
    }

    /// The bar this width across, laid out in a real context so a pointer can
    /// be aimed at it, and `frame` a pass of input through it.
    const W: f32 = 400.0;

    /// Where a value on the axis falls, as a fraction of the bar's own width —
    /// what the gesture harness below takes its two ends in.
    fn axis(v: f32) -> f32 {
        (HANDLE_INSET + (W - 2.0 * HANDLE_INSET) * v / QUAD_MARGIN) / W
    }

    /// Run `events` through a fresh context showing the bar, and answer where
    /// the bar came out. A context of its own per gesture, since egui's temp
    /// store — where the grab lives — has no expiry.
    fn gesture(view: &mut ViewConfig, passes: impl Fn(egui::Rect) -> Vec<Vec<egui::Event>>) {
        let ctx = crate::tests::probe::themed();
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(W, 100.0));
        let bar = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut t = 0.0;
        let mut frame = |view: &mut ViewConfig, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(t),
                    events,
                    ..Default::default()
                },
                |ui| bar.set(StackBar::new(view, "Layers").show(ui).rect),
            );
        };
        // A frame with no input first: egui resolves the pointer against the
        // PREVIOUS pass's widget rects, so the bar has to have been laid out
        // once before a press can land on it.
        frame(view, vec![]);
        for events in passes(bar.get()) {
            frame(view, events);
        }
    }

    /// Drag the bar from `from` to `to`, both fractions of its width.
    fn drag(view: &mut ViewConfig, (from, to): (f32, f32)) {
        gesture(view, |bar| {
            let at = |x: f32| egui::pos2(bar.left() + bar.width() * x, bar.center().y);
            // A step clear of egui's drag threshold first, then the rest of the
            // way, because that is what a real hand delivers: the first frame
            // the bar sees is never at `from`, which is the gap `aimed_at`
            // exists for.
            let step = 12.0 / bar.width() * (to - from).signum();
            vec![
                vec![egui::Event::PointerMoved(at(from))],
                vec![egui::Event::PointerMoved(at(from)), press(at(from), true)],
                vec![egui::Event::PointerMoved(at(from + step))],
                vec![egui::Event::PointerMoved(at(to))],
            ]
        });
    }

    /// A drag moves the layer it was aimed at and no other — the claim the grab
    /// machinery exists for, made through a real pointer rather than through
    /// `aimed` alone.
    #[test]
    fn a_drag_moves_only_the_layer_it_was_aimed_at() {
        let before = fresh();
        // Aimed at the octave band's own body: the stretch of bar between the
        // audio ring's boundary and the strip's.
        let rings = before.rings();
        let mid = axis((rings.band.0 + rings.band.1) * 0.5);
        let mut after = before.clone();
        drag(&mut after, (mid, mid + 0.08));
        assert!(
            after.band_width > before.band_width,
            "the band did not widen: {} -> {}",
            before.band_width,
            after.band_width,
        );
        assert_eq!(after.core_radius, before.core_radius, "the core moved");
        assert_eq!(after.spectral_ring_width, before.spectral_ring_width, "the audio ring moved");
        assert_eq!(after.mark_thickness, before.mark_thickness, "the mark strip moved");
    }

    /// The strip's outer edge is a handle of its own, out past the three the
    /// rings answer to — so the marks are sized on this bar rather than left
    /// behind on one of their own.
    #[test]
    fn the_outermost_handle_sizes_the_mark_strip() {
        let before = fresh();
        let rings = before.rings();
        let end = axis(rings.mark_inner + rings.mark_thickness);
        let mut after = before.clone();
        drag(&mut after, (end, end + 0.06));
        assert!(
            after.mark_thickness > before.mark_thickness,
            "the strip did not deepen: {} -> {}",
            before.mark_thickness,
            after.mark_thickness,
        );
        assert_eq!(after.band_width, before.band_width, "the band moved with it");
    }

    /// Double-click puts the four sizes back where a fresh view opens them.
    /// [`ValueBar`]'s type-a-value gesture is no use to a bar holding four
    /// values, so this is what the gesture is spent on.
    ///
    /// [`ValueBar`]: super::super::value::ValueBar
    #[test]
    fn double_click_restores_the_fresh_stack() {
        let mut view = fresh();
        for layer in LAYERS {
            layer.set(&mut view, 0.0);
        }
        gesture(&mut view, |bar| {
            let at = bar.center();
            vec![
                vec![egui::Event::PointerMoved(at)],
                vec![press(at, true)],
                vec![press(at, false)],
                vec![press(at, true)],
                vec![press(at, false)],
            ]
        });
        for layer in LAYERS {
            assert_eq!(
                layer.width(&view),
                layer.width(&fresh()),
                "{layer:?} did not come home on a double-click",
            );
        }
    }
}
