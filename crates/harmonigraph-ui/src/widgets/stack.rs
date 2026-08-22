//! [`StackBar`]: a node's cross-section in one bar — the empty middle it is
//! read out from and the three layers stacked around that, each a cell as long
//! as it is thick, named against the handle that opens it, with a handle
//! apiece to size it by.

use egui::{CornerRadius, Response, Sense, TextStyle, Ui, Vec2};
use harmonigraph_scene::{
    RingStack, ViewConfig, MARK_THICKNESS_MAX, RING_INNER_MAX, RING_WIDTH_MAX,
};

use super::bar::{
    aimed_at, bar_radius, bar_width, grabbed, grip_over_text, release_grab, track_fill,
    BAR_TEXT_PAD, HANDLE_INSET, HANDLE_W,
};
use crate::theme;

/// The top of the bar's axis, in the quad units the four sizes are in: the quad
/// edge, plus the deepest the melody/bass strip can be laid off it.
///
/// **Not the billboard's whole reach**, which is what a bar drawing "everywhere
/// a node draws" would run to and which spends two fifths of itself on room
/// nothing goes. A ring past the quad edge is refused, so 1.0 is where three of
/// the four layers stop for good, and the only thing out past it is a strip
/// that can be [`MARK_THICKNESS_MAX`] deep. A bar carrying anything past that
/// carries travel no handle can use, at the price of the length the layers'
/// names need: a fresh node ends at 0.94, which is under three quarters of
/// this axis and under two thirds of the billboard's.
///
/// What it costs is the corner where the rings are pushed hard against the quad
/// edge, which leaves the strip starting a gap PAST 1.0 with the last of its
/// depth off the end of the axis. The billboard's reach caps that same corner —
/// a gap at its own maximum puts the strip's outer edge at 1.7, past the 1.6
/// the shader eases it away by — so it is a trade an axis of any length here
/// makes, at a number that pays for it.
const AXIS_TOP: f32 = 1.0 + MARK_THICKNESS_MAX;

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

/// Which of a node's sizes a drag is moving: the middle it stands its stack
/// out from, or one of the three layers stacked around that.
///
/// The middle is not a layer — nothing is drawn in it — but it is one of the
/// bar's stretches, one of its handles and one number in the view, which is
/// everything this enum is asked. Where it parts company with the other three
/// is that its number is a RADIUS: see [`resized`], the one place that
/// difference is arithmetic rather than prose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Layer {
    #[default]
    Inner,
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
const LAYERS: [Layer; 4] = [Layer::Inner, Layer::Audio, Layer::Band, Layer::Mark];

impl Layer {
    fn index(self) -> usize {
        match self {
            Layer::Inner => 0,
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
            Layer::Inner => view.ring_inner,
            Layer::Audio => view.spectral_ring_width,
            Layer::Band => view.band_width,
            Layer::Mark => view.mark_thickness,
        }
    }

    fn set(self, view: &mut ViewConfig, width: f32) {
        *match self {
            Layer::Inner => &mut view.ring_inner,
            Layer::Audio => &mut view.spectral_ring_width,
            Layer::Band => &mut view.band_width,
            Layer::Mark => &mut view.mark_thickness,
        } = width;
    }
}

/// Where a drag on layer `k` leaves that layer's width, to put its boundary at
/// `to` on the axis of a node standing at `rings`, the layer being `current`
/// wide now.
///
/// **The whole of what a gesture on this bar writes**, so the pure tests below
/// and the pointer path in [`StackBar::show`] cannot come to disagree about
/// what a drag does. What actually matters — the boundary landing where the
/// gesture asks for it, the off position at the bottom of a layer's travel, a
/// ring stopping at the quad edge, and a refused layer keeping its width — is
/// then testable without a pointer.
///
/// **Every one of the four reads only the stack INSIDE it**: the middle reads
/// nothing, a ring its own slot's start, the marks theirs. So a gesture can
/// never re-read a number it moved itself, and the boundary it is dragging
/// stays a fixed distance from the width it is writing however far the drag
/// runs. A formula that read the boundary OUTSIDE the layer would re-measure a
/// stack the drag had already slid and creep while the pointer sat still.
///
/// The `pad` is what makes the arithmetic uniform across layers that are not
/// uniform. A boundary is one [`gap`](RingStack::gap) past the layer it closes,
/// because the next layer out stands off it — except at the two ends. The marks
/// close the stack and stand off nobody, so their boundary is flush with the
/// strip's own end; and the innermost ring stands off nothing either, a gap
/// being padding between two DRAWN layers and the middle being no layer at all,
/// so the middle's boundary is exactly the radius its handle names.
///
/// **Under a gap of travel a layer that stands one off reads 0 and switches
/// off**, which is the price of the stack closing up around a layer that is not
/// there: a width of nothing gives back its slot AND its padding, so the
/// boundary jumps a gap inward the moment the layer goes. Those three handles
/// have that gap of dead travel at the bottom rather than a step they could be
/// dragged across, and the alternative — sliding the boundary over ground the
/// picture will not draw it on — is the bar lying about where the layer inside
/// it ends. The marks stand nobody off and so have no such band: their strip
/// thins to nothing continuously.
fn resized(k: usize, to: f32, rings: &RingStack, current: f32) -> f32 {
    // A layer the stack REFUSED keeps what it is holding. Refusal is the room
    // running out from further in, so there is no cell on screen to move and
    // every thumb is piled on the last layer that fit: a write here would set a
    // size the picture cannot show, and the natural way to find out whether a
    // handle is live — pulling it inward — would land on 0 and destroy the
    // width the layer is keeping for when the room comes back. The one way back
    // is from the inside, by narrowing whatever took the room.
    //
    // Told from an OFF layer by the width it holds, not by the empty span the
    // two share: 0 is off and is the state a handle exists to leave.
    let (lo, hi) = layer_spans(rings)[k];
    if current > 0.0 && hi <= lo {
        return current;
    }
    let (edges, gap) = (rings.edges(), rings.gap);
    let (start, pad) = match k {
        0 => (0.0, 0.0),
        1 => (edges[0], gap),
        2 => (edges[1], gap),
        _ => (edges[2], 0.0),
    };
    // A ring's own drag stops at the quad edge, so a handle dragged to the end
    // of the axis leaves the widest layer that still FITS rather than one the
    // stack would refuse — which would drop the ring off the node at the top of
    // its own travel, and take every layer outside it along. Refusal is still
    // reachable, from the inside: push the middle out past what the ring needs
    // and the ring goes, which is the stack running out of room rather than a
    // bar asking for a size that never made sense.
    //
    // The marks are the one layer with no wall to stop at. They are drawn into
    // the billboard's margin past the quad, which is what the axis's last
    // stretch is (see [`AXIS_TOP`]).
    //
    // `min`/`max` rather than a clamp against a computed floor: a stack whose
    // rings are already past the quad hands in a `start` above 1, and
    // `f32::clamp` asserts `min <= max` and takes the editor down with it.
    let high = match k {
        0 => RING_INNER_MAX,
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
/// truth and the drift falls on the layers stacked onto it. A pile of all four
/// is the widest lie this can tell, three separations of a thumb's width
/// apiece, and it is the state a node with one layer left is genuinely in.
///
/// **The drift can land on a layer that IS drawn**, and the case is worth
/// naming because it is the one a node dialled down reaches: seat the middle on
/// the center and switch the audio ring and the band off, and the strip is the
/// innermost layer on, so
/// it starts at the node's center with three off boundaries piled on 0 in front
/// of it. Its own thumb is then pushed past its cell, and the presses over that
/// cell go to the three layers that are not there. What that buys is the only
/// thing this bar cannot do without — every layer reachable from its own handle
/// — and what it costs is a press on the last visible layer taking a layer
/// inside it. The alternative is a node with one layer left that can never grow
/// a second.
///
/// The innermost is pushed clear of the bar's own end as well, so a layer sized
/// to nothing at the bottom of the axis is still something to take hold of.
/// That is the widest the drift ever is at rest — a separation out of the
/// track, under two hundredths of the axis — and what it buys is the gesture
/// that pushes the middle back out.
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
/// By REGION and not by the nearer thumb, which is what gives a layer a stretch
/// of bar that means it — its own body, and the gap it stands off by — so
/// pressing on a ring and dragging is that ring widening. A nearest-thumb rule
/// would hand the inner half of every layer to the boundary inside it, and a
/// press on the middle of the audio ring would move the node's own middle.
///
/// The regions follow the thumbs where [`spread`] pushes them, not where the
/// boundaries are, so a layer whose thumb was pushed off its own cell has a
/// region that does not cover it. That is the same bargain read from the other
/// end, and `spread` is where it is argued.
///
/// The empty track past the outermost thumb belongs to the marks, which is the
/// layer a press out there is aiming at: it is the only one whose room is out
/// there.
///
/// A thumb stands in the middle of the gap its layer holds open (see
/// [`thumb_axis`]), so a layer's stretch is its own cell plus half the gap at
/// each end of it — the padding a layer keeps splitting between the two layers
/// it stands between, which is the only division of it that is not a choice.
fn aimed(x: f32, thumbs: [f32; 4], half_thumb: f32) -> Layer {
    LAYERS
        .iter()
        .zip(thumbs)
        .find(|(_, thumb)| x <= thumb + half_thumb)
        .map_or(Layer::Mark, |(layer, _)| *layer)
}

/// The four stretches as the stack DREW them, innermost first: each one's inner
/// and outer radius, and an empty pair for one that is not on the node.
///
/// A layer is the width its handle reads or it is not on the node at all, so an
/// empty pair is the one test for "this layer is here" and there is no second
/// flag that could come to disagree with it. The middle answers the same way
/// and means something a shade different by it: an empty pair there is a stack
/// seated on the node's own center, which is a picture rather than an absence.
fn layer_spans(rings: &RingStack) -> [(f32, f32); 4] {
    [
        (0.0, rings.inner),
        rings.audio,
        rings.band,
        (rings.mark_inner, rings.mark_inner + rings.mark_thickness),
    ]
}

/// Where each thumb stands on the axis: the MIDDLE of the gap its layer holds
/// open, rather than the boundary at the far side of that gap.
///
/// A layer's boundary is one [`gap`](RingStack::gap) out from where the layer
/// itself stopped, so a thumb standing honestly on it sits flush against the
/// NEXT cell's leading edge and reads as belonging to the layer it does not
/// size. Half a gap in, it stands on bare track between two cells and touches
/// neither, which is what a divider looks like — and it is the same place the
/// eye already reads the boundary as being.
///
/// **The value is unchanged**: [`resized`] still answers with the boundary, and
/// the press's own distance from it is what the gesture holds ([`Grab`]), so
/// the thumb travels with the pointer however far the truth is from where the
/// thumb was drawn. That is [`RangeBar`]'s bargain — a handle drawn where it
/// reads rather than where its value is — at half a gap.
///
/// The marks close the stack and stand nobody off, so there is no gap out there
/// and their thumb is on the strip's own outer edge. Neither is there one in
/// front of the innermost ring, which seats straight onto the middle, so that
/// thumb is on the middle's own edge — flush against the ring it opens, which
/// is what a boundary with no padding at it looks like. A layer that is OFF has
/// no gap either, its boundary having collapsed onto the one inside it, and its
/// thumb goes wherever [`spread`] can still find room for it.
///
/// [`RangeBar`]: super::range::RangeBar
fn thumb_axis(rings: &RingStack) -> [f32; 4] {
    let edges = rings.edges();
    let mut out = edges;
    for (k, (lo, hi)) in layer_spans(rings).into_iter().enumerate() {
        if hi > lo {
            out[k] = (hi + edges[k]) * 0.5;
        }
    }
    out
}

/// What each layer is called on the bar, innermost first: the Lattice page's
/// own sections, cut to the one word that tells them apart.
///
/// Short because the room a name gets is a layer's own thickness plus whatever
/// its neighbours have not spent on their own names, and at a fresh view three
/// of the four layers are under a fifth of the axis. "Melody / bass" would be
/// off the bar at every pane width, and a name with nowhere clear to go is not
/// drawn at all — so every word here is chosen to be the longest one that still
/// lands.
///
/// **MIDI rather than the "Octaves" heading it sits under**, which is the one
/// place this parts company with the pane, and it says the thing the bar is
/// asked: the two rings in the middle of a node are the same annulus drawn
/// twice, and what tells them apart is where each one's reading comes FROM —
/// the analyzer's spectrum on the inner one, the played notes on the outer. So
/// Audio and MIDI are one pair, read together. "Octaves" names the pitch axis
/// drawn on that layer rather than the layer, and is a word too long for the
/// narrowest stretch on the bar besides.
const NAMES: [&str; 4] = ["Inner", "Audio", "MIDI", "Marks"];

/// The four sizes of a node's layer stack in one bar, drawn as the node's own
/// cross-section: the empty middle out from the center, the audio ring, the
/// octave band and the melody/bass strip, each a cell as long as it is thick and
/// carrying its own name, with the Ring gap standing between them as bare track.
///
/// **One control rather than four, because the four are not independent
/// numbers.** Each layer's inner edge is a sum over every layer inside it, so a
/// bar apiece asking for a width can only be read one at a time and none of
/// them says where its layer lands — the one question a size on a node is
/// asked. Here the answer is the picture, and the drag is the same gesture the
/// reading is: a handle is a layer's outer edge, and pulling it out is that
/// layer getting thicker.
///
/// **Four handles for four layers**, each standing at the boundary its layer
/// ends on — the audio ring's inner radius closes the middle, the band's closes
/// the ring, the strip's closes the band, and the strip's own outer edge closes
/// the strip. So each handle sizes the layer INSIDE it and slides everything
/// outside it along, which is exactly what the stack does on screen
/// ([`ViewConfig::rings`]).
///
/// **The axis runs a little past the quad edge and no further** ([`AXIS_TOP`]).
/// That edge sits inside it, marked by a hairline, and it is a real wall rather
/// than a scale mark — a RING that no longer fits inside it is refused rather
/// than clipped, so the layers drop off the outside of the stack one at a time
/// as the room runs out. The stretch past it is the billboard's margin, which
/// the marks alone are allowed into, and the axis carries exactly as much of it
/// as the strip can be deep.
///
/// **0 is every layer's off position**, reached by dragging its handle back
/// onto the one inside it, and the gaps go with it: a layer at 0 gives up its
/// slot and its padding together, so the bar shows the stack closing up rather
/// than a hole where the layer was.
///
/// **Each layer is named rather than numbered**, and the bar carries no name of
/// its own: a row of four numbers says how thick each layer is and never which
/// layer is which, where a name lying on the layer says both at once. So the
/// row leads with "Inner" where its neighbours lead with their own names, in
/// the same place, and what follows it along the bar is the rest of the node.
///
/// A layer too thin to hold its own name is named against the handle that
/// OPENS it instead, on whichever side of that handle has room — see the
/// placement in [`StackBar::show`]. At a fresh view that is three of the four,
/// so it is the ordinary case here rather than the corner: the length of a name
/// is then the name's own and says nothing, and it is the picture beside it
/// that says how thick.
///
/// Double-click restores the four sizes a fresh view opens with. It is not
/// [`ValueBar`]'s type-a-value gesture, for [`RangeBar`]'s reason doubled: a
/// bar with four values has no single one to type into it.
///
/// [`ValueBar`]: super::value::ValueBar
/// [`RangeBar`]: super::range::RangeBar
pub struct StackBar<'a> {
    view: &'a mut ViewConfig,
}

impl<'a> StackBar<'a> {
    pub fn new(view: &'a mut ViewConfig) -> Self {
        StackBar { view }
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
        // here than on a range: a node with nothing but its middle parks three
        // thumbs at the bottom of the axis, and a fresh view already stands the
        // outermost at 0.94 of the quad.
        let inset = HANDLE_INSET * scale;
        let track = rect.shrink2(Vec2::new(inset, 0.0));
        let x_of = |v: f32| track.left() + track.width() * (v / AXIS_TOP).clamp(0.0, 1.0);
        let value_at = |x: f32| {
            ((x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0) * AXIS_TOP
        };
        let sep = THUMB_SEP * scale;
        let half_thumb = HANDLE_W * 0.5 * scale;
        let thumbs_of = |rings: &RingStack| {
            spread(thumb_axis(rings).map(&x_of), (track.left(), track.right()), sep)
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
                let current = grab.layer.width(self.view);
                let want = resized(grab.layer.index(), to, &rings, current);
                if want != current {
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
        let spans = layer_spans(&rings);
        // A cell standing at the node's CENTER is drawn out to the bar's own
        // end, past the inset the axis keeps: that inset is room for a thumb to
        // seat in at either limit, and a layer whose inner radius is 0 starting
        // a few points along would read as a gap in front of the node's center
        // — a place a node has no room to leave. The layer is the innermost one
        // ON, which is the middle until the middle is seated on the center.
        let cells = spans.map(|(lo, hi)| {
            let left = if lo <= 0.0 { rect.left() } else { x_of(lo) };
            egui::Rect::from_x_y_ranges(left..=x_of(hi), rect.y_range())
        });
        let r = bar_radius(scale);
        let radius = CornerRadius::same(r);
        let painter = ui.painter();
        painter.rect_filled(rect, radius, theme::well());

        let fill = track_fill(&response);
        // The strip drawn in the plain widget fill when neither end is marked:
        // its depth is still this bar's to set, and the node still keeps the
        // room, but nothing is wearing it. The alternative is greying the whole
        // bar on a checkbox two sections down, which would take the other three
        // layers with it.
        //
        // The gaps between the cells are the track itself, which is what the
        // empty run past the outermost layer is too. One ground for both, since
        // both are the same thing — node the picture does not draw on — and a
        // shade laid over the padding would make the Ring gap a fifth thing on
        // the bar rather than the spacing between four.
        //
        // The middle is drawn in that same plain fill for the same reason and
        // permanently: the node keeps the room and nothing is ever drawn in it,
        // what fills it on screen being the node's own light rather than a
        // layer. A cell rather than bare track, because the track means the
        // padding BETWEEN two layers — a spacing nothing sets directly — and
        // this is a size with a handle on it.
        let marked = self.view.mark_melody || self.view.mark_bass;
        for (i, cell) in cells.into_iter().enumerate() {
            let (lo, hi) = spans[i];
            if hi > lo {
                let empty = i == 0 || (i == 3 && !marked);
                painter.rect_filled(cell, radius, if empty { theme::widget() } else { fill });
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

        // Every layer that is on the node wears its own name, laid at the
        // handle that OPENS it — the inner end of the stretch of bar that means
        // it, which is the same stretch a press there takes hold of ([`aimed`]).
        //
        // The innermost name is held off the bar's end by the inset every other
        // row in the pane holds its own name by, so the row leads with "Inner"
        // where the row above leads with "Fade curve", on the same column of
        // pixels. The rest stand off the thumb that opens their stretch by
        // enough to clear it.
        //
        // **A name whose own stretch is too short for it hangs BACK off that
        // handle instead**, ending the same pad short of it that a name which
        // fits starts past it. Only when there is no room behind either does it
        // spill FORWARD past its own layer, and then only while its middle is
        // still inside its own stretch.
        //
        // Which is a name leaving the stretch it names, and it is worth what it
        // costs. The room on this bar is not shared out the way the names need
        // it: a fresh node spends over a third of the axis on the empty middle,
        // whose name wants a fifth of that, and gives the audio ring a
        // twentieth, which is under half what "Audio" wants at ANY width a
        // settings column reaches (#405). Naming only what fits leaves that
        // ring — the layer most worth naming, being the one with no room to
        // say what it is — anonymous for good, while the bar carries the length
        // its name needs a few pixels away.
        //
        // **What both borrowings are bounded by is INK, and that bound is the
        // whole of what keeps them readable.** A name is read against whatever
        // it is lying on, so one laid across another layer's CELL names that
        // layer, whatever the order says — the failure is not a name going
        // missing but a name going to the wrong ring, which is worse than the
        // anonymity it was spent to buy. So a name reaches back only over bar
        // with nothing drawn in it, which is the empty middle and the track
        // between cells; and it spills forward only as far as its own middle,
        // past which the cell it covers most is no longer its own. Between
        // those, the names stay in the stack's order and never touch, so the
        // third name from the left is the third layer out.
        //
        // A name with nowhere clear to go goes WITHOUT rather than eliding: an
        // ellipsis costs most of the room a four-letter name needs, and a layer
        // dialled down to a sliver would spend its whole cell saying nothing.
        // What the bar always shows is the picture.
        let text_color = if response.hovered() || response.dragged() {
            theme::text()
        } else {
            theme::text_dim()
        };
        let thumbs = thumbs_of(&rings);
        let body = TextStyle::Body.resolve(ui.style());
        let mut runs: Vec<(egui::Pos2, std::sync::Arc<egui::Galley>)> = Vec::new();
        // The right edge of the last name DRAWN, so a name that went without
        // leaves its room to the next one rather than holding it empty.
        let mut cursor = rect.left();
        // And the right edge of the last cell with INK in it — how far back a
        // name may reach, per the paragraph above. The middle is drawn but
        // carries none, which is exactly why it is the room the audio ring's
        // name is found in; the strip carries none either while neither end is
        // marked, on the same terms the cell loop greys it by.
        let mut inked = rect.left();
        for (i, (lo, hi)) in spans.into_iter().enumerate() {
            if hi <= lo {
                continue;
            }
            let from = if i == 0 { rect.left() } else { thumbs[i - 1] };
            let to = if i == 3 { rect.right() } else { thumbs[i] };
            let pad = if i == 0 { BAR_TEXT_PAD } else { HANDLE_INSET } * scale;
            let name = painter.layout_no_wrap(NAMES[i].to_owned(), body.clone(), text_color);
            let w = name.size().x;
            // Past the handle that opens the layer, or past the name before it
            // where that reaches further — the pad being the room a name keeps
            // in front of itself, whether what stands there is a thumb or
            // another name.
            let ahead = from.max(cursor) + pad;
            let back = from - pad - w;
            let placed = if ahead + w <= to {
                // Inside its own stretch, which is where a name belongs.
                Some(ahead)
            } else if back >= cursor.max(inked) + pad {
                // Behind the handle that opens it, over bar carrying no ink.
                Some(back)
            } else if ahead + 0.5 * w <= to {
                // Forward, while more of the name is still its layer's than
                // the next one's.
                Some(ahead)
            } else {
                None
            };
            if let Some(x) = placed.filter(|x| x + w <= rect.right()) {
                let pos = egui::pos2(x, rect.center().y - name.size().y * 0.5);
                painter.galley(pos, name.clone(), text_color);
                runs.push((pos, name));
                cursor = x + w;
            }
            // After this layer's own name is placed: a name is never held off
            // the cell it names, only off the cells INSIDE it.
            if !(i == 0 || (i == 3 && !marked)) {
                inked = cells[i].right();
            }
        }

        // The thumbs last, over the text: they are the part you operate, and a
        // letter sliding under one is a better outcome than a handle
        // disappearing behind a letter. Every run is knocked back out through
        // the grip, none of them being placeable clear of four thumbs roaming
        // one row — a name is pinned to its own cell, and the thumb that sizes
        // that cell crosses it whenever the layer is dialled down to about the
        // width of its name.
        let grip_radius = CornerRadius::same(theme::scaled_points(2, scale));
        for x in thumbs {
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
    use crate::widgets::probe::{filled_rects, handles, press, shapes, text_boxes};

    /// A fresh view, which is the stack every claim here is measured against
    /// unless it says otherwise.
    fn fresh() -> ViewConfig {
        ViewConfig::default()
    }

    /// `view` with layer `k`'s handle dragged to `v` up the axis.
    fn dragged(view: &ViewConfig, k: usize, v: f32) -> ViewConfig {
        let mut out = view.clone();
        let rings = out.rings();
        let width = resized(k, v, &rings, LAYERS[k].width(&out));
        LAYERS[k].set(&mut out, width);
        out
    }

    /// A layer the stack REFUSED keeps the width it is holding when its handle
    /// is dragged, rather than losing it to a gesture nothing answers.
    ///
    /// Refusal is the room running out from further in, so there is no cell to
    /// move and no boundary to move it to: every one of the four thumbs is
    /// piled on the last layer that fit. A write there would set a size against
    /// a picture that cannot show it — and dragging inward, the natural way to
    /// find out whether a handle is live, sets it to 0 and destroys the width
    /// the layer was keeping for when the room came back.
    #[test]
    fn a_refused_layer_keeps_its_width_when_its_handle_is_dragged() {
        let mut view = fresh();
        // The stack pushed to the edge of the quad with a ring too wide to seat
        // there, so the audio ring no longer fits and takes the band with it:
        // the stack drops from the outside in and stays dropped.
        view.ring_inner = 0.95;
        view.spectral_ring_width = 0.2;
        let rings = view.rings();
        assert!(rings.audio.1 <= rings.audio.0, "the audio ring was meant to be refused here");
        assert!(rings.band.1 <= rings.band.0, "the band was meant to go with it");
        for k in [1, 2] {
            let home = dragged(&view, k, rings.edges()[k] - 0.2);
            assert_eq!(
                LAYERS[k].width(&home),
                LAYERS[k].width(&view),
                "layer {k} lost the width it was keeping while the stack had no room for it",
            );
            let out = dragged(&view, k, rings.edges()[k] + 0.2);
            assert_eq!(
                LAYERS[k].width(&out),
                LAYERS[k].width(&view),
                "layer {k} was resized by a handle with nothing on screen to move",
            );
        }
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
        // As far out as the middle can go with every layer outside it still
        // fitting; past that the stack starts refusing them, which is the next
        // test.
        let moved = dragged(&view, 0, 0.4);
        for layer in &LAYERS[1..] {
            assert_eq!(
                layer.width(&moved),
                layer.width(&view),
                "{layer:?} lost width to the middle being pushed out",
            );
        }
        let (before, after) = (view.rings().edges(), moved.rings().edges());
        for k in 1..4 {
            assert!(
                (after[k] - before[k] - (after[0] - before[0])).abs() < 1e-5,
                "layer {k}'s boundary did not slide with the middle's",
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
            let out = dragged(&view, k, AXIS_TOP);
            let (lo, hi) = if k == 1 { out.rings().audio } else { out.rings().band };
            assert!(hi > lo, "layer {k} dragged to the end of the axis came off the node");
            assert!(hi <= 1.0 + 1e-6, "layer {k} was left reaching {hi}, past the quad edge");
        }
    }

    /// A slot that starts PAST the quad edge leaves the layer no room, rather
    /// than a negative ceiling — which is the state `resized` takes its `min`
    /// and `max` in place of a clamp for, `f32::clamp` asserting `min <= max`
    /// and taking the editor down with it from the paint path.
    ///
    /// Reachable from the bars alone, which is what makes it worth a fixture:
    /// the middle at its own maximum, an audio ring that just reaches the quad
    /// edge from there, and the Ring gap at its own maximum puts the band's slot
    /// at 1.4. The band is OFF there rather than refused, so the guard above
    /// does not stand in front of this one.
    #[test]
    fn a_slot_starting_past_the_quad_edge_leaves_no_room() {
        let mut view = fresh();
        view.ring_inner = RING_INNER_MAX;
        view.spectral_ring_width = 1.0 - RING_INNER_MAX;
        view.ring_gap = harmonigraph_scene::GAP_MAX;
        view.band_width = 0.0;
        let rings = view.rings();
        assert!(
            rings.edges()[1] > 1.0,
            "the band's slot was meant to start past the quad edge, not at {}",
            rings.edges()[1],
        );
        assert_eq!(
            resized(2, AXIS_TOP, &rings, 0.0),
            0.0,
            "a slot with no room left was given a width anyway",
        );
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
        let far = dragged(&view, 0, 0.8);
        let rings = far.rings();
        assert!(rings.band.1 <= rings.band.0, "the band was kept at a width it had no room for");
        assert_eq!(
            far.band_width, view.band_width,
            "the band's own size was written by the middle it stands outside",
        );
        assert!(rings.audio.1 > rings.audio.0, "the audio ring went with it, from further in");
    }

    /// The marks are the one layer with no such wall — the axis's last stretch
    /// is the billboard margin they are drawn into — and the axis carries
    /// enough of that margin to reach the deepest strip a node can wear, which
    /// is the whole of what its length is for.
    #[test]
    fn the_marks_reach_their_full_depth_past_the_quad_edge() {
        let view = fresh();
        let out = dragged(&view, 3, AXIS_TOP);
        let rings = out.rings();
        assert!(
            rings.mark_inner + rings.mark_thickness > 1.0,
            "the strip was held inside the quad at {}",
            rings.mark_inner + rings.mark_thickness,
        );
        assert_eq!(
            out.mark_thickness, MARK_THICKNESS_MAX,
            "the end of the axis left the strip short of its own maximum",
        );
    }

    /// Every one of the four thumbs can be pressed, on a node with nothing left
    /// on it at all — the state where all four boundaries stand on one point,
    /// and the state a bar that placed them honestly could never be dragged out
    /// of, no handle on it having any bar of its own to be pressed.
    ///
    /// Derived from the view rather than from a pile written down here, so what
    /// it holds is the whole chain a press goes through — the boundaries, where
    /// they fall on the axis, the spreading and the region split. A hardcoded
    /// pile would test `spread` and `aimed` against each other and say nothing
    /// about whether a node can reach that state or what its thumbs do there.
    #[test]
    fn a_pile_of_thumbs_still_answers_four_presses() {
        let mut view = fresh();
        for layer in LAYERS {
            layer.set(&mut view, 0.0);
        }
        let rings = view.rings();
        assert_eq!(
            thumb_axis(&rings),
            [0.0; 4],
            "a node with no layers was meant to pile all four thumbs at its center",
        );
        let thumbs = spread(
            thumb_axis(&rings).map(|v| axis(v) * W),
            (axis(0.0) * W, axis(AXIS_TOP) * W),
            THUMB_SEP,
        );
        let taken: Vec<Layer> =
            thumbs.iter().map(|&x| aimed(x, thumbs, HANDLE_W * 0.5)).collect();
        assert_eq!(taken, LAYERS.to_vec(), "a press on each thumb did not take each layer");
    }

    /// The strip's cell draws in the plain widget fill when neither end is
    /// ticked: sized, with the node keeping the room, but nothing wearing it.
    /// Greying the bar instead would take the other three layers with it, on a
    /// pair of checkboxes two sections down that say nothing about them.
    #[test]
    fn an_unmarked_strip_draws_in_the_plain_widget_fill() {
        let mut view = fresh();
        view.mark_melody = false;
        view.mark_bass = false;
        let rings = view.rings();
        assert!(rings.mark_thickness > 0.0, "the strip is still sized with neither end marked");
        let shapes = shapes(W, |ui| {
            StackBar::new(&mut view).show(ui);
        });
        let (_, x_of) = axis_on(&shapes);
        let (_, fill) = filled_rects(&shapes)
            .into_iter()
            .find(|(r, _)| (r.left() - x_of(rings.mark_inner)).abs() < 0.5)
            .expect("no cell was drawn for the strip");
        assert_eq!(fill, theme::widget(), "the strip took the accent with nothing wearing it");
    }

    /// A press inside a layer's own stretch of bar takes that layer, not the
    /// boundary nearest it — pressing on the middle of the audio ring and
    /// dragging is the ring widening.
    #[test]
    fn a_press_on_a_layer_takes_that_layer() {
        let thumbs = [40.0, 120.0, 200.0, 260.0];
        for (x, want) in [
            (10.0, Layer::Inner),
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

    /// The bar's own rect out of a paint list, and where a value on the axis
    /// falls across it — the reading every claim about the picture is made in.
    fn axis_on(shapes: &[egui::Shape]) -> (egui::Rect, impl Fn(f32) -> f32) {
        let bar = filled_rects(shapes).first().expect("the bar drew no track").0;
        let inner = bar.shrink2(egui::vec2(HANDLE_INSET, 0.0));
        (bar, move |v: f32| inner.left() + inner.width() * v / AXIS_TOP)
    }

    /// The bar paints one cell per layer that is on the node, at the radii the
    /// stack put them at, and nothing at all between them: a gap is the track
    /// showing through, which is what the empty run past the last layer is too.
    #[test]
    fn the_cells_are_the_stack_the_node_draws() {
        let mut view = fresh();
        let shapes = shapes(W, |ui| {
            StackBar::new(&mut view).show(ui);
        });
        let rings = fresh().rings();
        let fills = filled_rects(&shapes);
        let (_, x_of) = axis_on(&shapes);
        for (lo, hi) in [rings.audio, rings.band] {
            assert!(
                fills.iter().any(|(r, _)| (r.left() - x_of(lo)).abs() < 0.5
                    && (r.right() - x_of(hi)).abs() < 0.5),
                "no cell was drawn for the layer at {lo}..{hi}",
            );
        }
        // Every fill but the track and the four thumbs, which is what the cells
        // have to be: a run of shading between two of them would be a fifth
        // thing on a bar that draws four layers.
        let cells = fills.iter().skip(1).filter(|(r, _)| r.width() > HANDLE_W).count();
        assert_eq!(cells, 4, "the bar drew something over its track besides its four cells");
    }

    /// The innermost cell reaches the bar's own end, past the inset the axis
    /// keeps for its thumbs: a node has no room to leave in front of its
    /// center, so a cell starting at 0 starts at the end of the bar.
    #[test]
    fn the_cell_at_the_nodes_center_reaches_the_end_of_the_bar() {
        let mut view = fresh();
        let shapes = shapes(W, |ui| {
            StackBar::new(&mut view).show(ui);
        });
        let fills = filled_rects(&shapes);
        let (bar, x_of) = axis_on(&shapes);
        let middle = fills
            .iter()
            .skip(1)
            .find(|(r, _)| (r.right() - x_of(fresh().ring_inner)).abs() < 0.5)
            .expect("no cell was drawn for the node's middle")
            .0;
        assert!(
            (middle.left() - bar.left()).abs() < 0.5,
            "the middle's cell started at {} where the bar starts at {}",
            middle.left(),
            bar.left(),
        );
    }

    /// A thumb stands in the MIDDLE of the gap its layer holds open rather than
    /// on the boundary at the far side of it — a divider between two cells,
    /// touching neither, instead of the leading edge of the cell it does not
    /// size.
    #[test]
    fn a_thumb_stands_in_the_gap_its_layer_holds_open() {
        let rings = fresh().rings();
        assert!(rings.gap > 0.0, "a node with no padding has no gap to stand a thumb in");
        let spans = layer_spans(&rings);
        let thumbs = thumb_axis(&rings);
        // The innermost ring seats straight onto the middle, so there is no gap
        // in front of it either and its thumb is on the middle's own edge.
        assert_eq!(
            thumbs[0], spans[0].1,
            "the innermost thumb stood off a middle it holds no gap against",
        );
        for k in 1..3 {
            assert!(
                (thumbs[k] - (spans[k].1 + rings.gap * 0.5)).abs() < 1e-6,
                "layer {k}'s thumb stood at {} rather than half a gap out from {}",
                thumbs[k],
                spans[k].1,
            );
        }
        // The marks close the stack and stand nobody off, so there is no gap
        // out there to stand in.
        assert_eq!(thumbs[3], rings.edges()[3], "the outermost thumb left the strip's edge");
    }

    /// And a layer that is OFF holds no gap open either, so its thumb is back
    /// on its own boundary with the ones inside it — the pile the spreading
    /// exists to split.
    #[test]
    fn an_off_layer_keeps_its_thumb_on_its_boundary() {
        let mut view = fresh();
        Layer::Audio.set(&mut view, 0.0);
        let rings = view.rings();
        assert_eq!(
            thumb_axis(&rings)[1],
            rings.edges()[1],
            "the audio ring's thumb kept half a gap it no longer holds open",
        );
    }

    /// The innermost name is held off the bar's end by the inset every other
    /// row in the pane holds its own name by, so the row leads where its
    /// neighbours lead rather than a few points along from them.
    #[test]
    fn the_innermost_name_leads_where_every_other_row_leads() {
        let mut view = fresh();
        let shapes = shapes(W, |ui| {
            StackBar::new(&mut view).show(ui);
        });
        let runs = text_boxes(&shapes);
        let (middle, _) = runs
            .iter()
            .find(|(_, s)| s == NAMES[0])
            .unwrap_or_else(|| panic!("the middle's cell went unnamed: {runs:?}"));
        let (bar, _) = axis_on(&shapes);
        assert!(
            (middle.left() - (bar.left() + BAR_TEXT_PAD)).abs() < 0.5,
            "the middle's name started at {} rather than {} in from the bar's end",
            middle.left() - bar.left(),
            BAR_TEXT_PAD,
        );
    }

    /// Every width the bar is drawn at, from the narrowest a settings column
    /// can be dragged to up past the widest anyone opens one to. The name
    /// placement borrows room from its neighbours, so the widths where it goes
    /// wrong are not the ones anybody would think to sample: a fresh view
    /// mislabelled the octave band for the whole band 93..192 while reading
    /// correctly at 200, 400 and 600 (#405).
    fn every_column_width() -> impl Iterator<Item = f32> {
        (80u16..=700).map(f32::from)
    }

    /// Each layer's CELL at `w`, as an x range on the bar — the cells rather
    /// than the stretches, because a name is read against what it is lying ON,
    /// which is what the placement in [`StackBar::show`] is bounded by.
    ///
    /// The middle's cell runs from the bar's own end, the way the bar draws it.
    fn cells_on(shapes: &[egui::Shape], view: &ViewConfig) -> [Option<(f32, f32)>; 4] {
        let (bar, x_of) = axis_on(shapes);
        let spans = layer_spans(&view.rings());
        std::array::from_fn(|k| {
            let (lo, hi) = spans[k];
            (hi > lo).then(|| (if lo <= 0.0 { bar.left() } else { x_of(lo) }, x_of(hi)))
        })
    }

    /// No name in `shapes` covers another layer's cell more than it covers its
    /// own — the claim the bar lives or dies on, asked of one painting of it.
    ///
    /// A name is read against the cell it is lying on, so one laid across the
    /// octave band names the octave band whatever the stack order says — and a
    /// name on the WRONG ring is worse than the anonymity borrowing was spent
    /// to buy. That is the trade this refuses to make (#405), and it is what
    /// bounds both directions a name is allowed to borrow in.
    ///
    /// Covering none of either is allowed, and is the ordinary case for a name
    /// hung back over the empty middle: the middle carries no ink, so a name
    /// laid there has taken nothing from anybody. What is refused is a name
    /// that has left its own ring and landed on a neighbour's.
    fn no_name_lands_on_another_layer(w: f32, shapes: &[egui::Shape], view: &ViewConfig) {
        let cells = cells_on(shapes, view);
        let over = |run: &egui::Rect, cell: Option<(f32, f32)>| {
            cell.map_or(0.0, |(lo, hi)| (run.right().min(hi) - run.left().max(lo)).max(0.0))
        };
        for (run, name) in text_boxes(shapes) {
            let k = NAMES
                .iter()
                .position(|n| *n == name)
                .unwrap_or_else(|| panic!("the bar drew a run that is no layer's name: {name:?}"));
            let own = over(&run, cells[k]);
            for (j, cell) in cells.iter().enumerate() {
                // The middle is drawn and EMPTY, so nothing is taken from it:
                // it is the one cell a name is free to lie across, and the room
                // the audio ring's name is found in.
                let other = if j == 0 { 0.0 } else { over(&run, *cell) };
                assert!(
                    j == k || other <= own + 0.5,
                    "at {w}, {name:?} lies over {other} points of {:?}'s cell and {own} \
                     of its own — it names the wrong layer",
                    NAMES[j],
                );
            }
        }
    }

    /// At a fresh view, at every width a settings column can be dragged to.
    #[test]
    fn a_name_never_covers_another_layers_cell_more_than_its_own() {
        for w in every_column_width() {
            let mut view = fresh();
            let shapes = shapes(w, |ui| {
                StackBar::new(&mut view).show(ui);
            });
            no_name_lands_on_another_layer(w, &shapes, &view);
        }
    }

    /// Where each name drawn is allowed to be: laid against the handle that
    /// OPENS its layer, on one side of it or the other, and never wholly past
    /// the layer it names.
    ///
    /// A name is allowed to leave its own stretch — that is what lets the audio
    /// ring be named at all (#405) — but only backwards onto its own near edge,
    /// or forwards while more of it is still inside the stretch than out. What
    /// it may never do is float: a run more than its own pad short of where its
    /// layer begins, or one whose middle is past where the layer ends, names
    /// the layer beside it instead.
    #[test]
    fn a_name_is_laid_against_the_layer_it_names() {
        for w in every_column_width() {
            let mut view = fresh();
            let shapes = shapes(w, |ui| {
                StackBar::new(&mut view).show(ui);
            });
            let (bar, x_of) = axis_on(&shapes);
            let thumbs = spread(
                thumb_axis(&fresh().rings()).map(&x_of),
                (x_of(0.0), x_of(AXIS_TOP)),
                THUMB_SEP,
            );
            for (run, name) in text_boxes(&shapes) {
                let k = NAMES.iter().position(|n| *n == name).unwrap_or_else(|| {
                    panic!("the bar drew a run that is no layer's name: {name:?}")
                });
                let from = if k == 0 { bar.left() } else { thumbs[k - 1] };
                let to = if k == 3 { bar.right() } else { thumbs[k] };
                let pad = if k == 0 { BAR_TEXT_PAD } else { HANDLE_INSET };
                assert!(
                    run.right() >= from - pad - 0.5 && run.center().x <= to + 0.5,
                    "at {w}, {name:?} was drawn at {:?}, off the {from}..{to} it names",
                    run.x_range(),
                );
            }
        }
    }

    /// And the names stay in the stack's own order, never touching. That is
    /// what makes a name that left its stretch still readable: the third name
    /// from the left is the third layer out however far any of them borrowed,
    /// so the row can be read off in one direction like the node it draws.
    ///
    /// Read off the bar left to right and NOT in paint order, which is the only
    /// version of this that can fail: the loop paints innermost first, so paint
    /// order is the stack's by construction and asking it that way is asking
    /// nothing. What the eye gets is the x order, and it is the placement that
    /// decides whether the two agree.
    #[test]
    fn the_names_read_out_in_the_stacks_own_order() {
        for w in every_column_width() {
            let mut view = fresh();
            let shapes = shapes(w, |ui| {
                StackBar::new(&mut view).show(ui);
            });
            let mut runs = text_boxes(&shapes);
            runs.sort_by(|a, b| a.0.left().total_cmp(&b.0.left()));
            let order: Vec<usize> = runs
                .iter()
                .map(|(_, s)| NAMES.iter().position(|n| n == s).unwrap())
                .collect();
            assert!(
                order.windows(2).all(|p| p[0] < p[1]),
                "at {w}, the bar reads left to right as {order:?}",
            );
            assert!(
                runs.windows(2).all(|p| p[0].0.right() <= p[1].0.left() + 0.5),
                "at {w}, two names overlap: {:?}",
                runs.iter().map(|(r, s)| (s, r.x_range())).collect::<Vec<_>>(),
            );
        }
    }

    /// A layer switched OFF wears no name and holds no room: the bar it would
    /// have taken goes to the layers outside it, and none of them ends up
    /// wearing its name over a ring that is still on.
    ///
    /// The audio ring is the one to switch off, being the layer the borrowing
    /// exists for. With it gone the octave band is the first ring on the node,
    /// and the middle's ink-free room — the room "Audio" was found in — is the
    /// band's to be named in instead. Nothing else here reaches a bar painted
    /// with a layer missing, so this is also the only test that runs the name
    /// loop's `continue` and the `cursor` it deliberately does not advance.
    #[test]
    fn an_off_layer_leaves_its_room_to_the_names_outside_it() {
        let mut view = fresh();
        view.spectral_ring_width = 0.0;
        for w in every_column_width() {
            let mut view = view.clone();
            let shapes = shapes(w, |ui| {
                StackBar::new(&mut view).show(ui);
            });
            let drawn: Vec<String> = text_boxes(&shapes).into_iter().map(|(_, s)| s).collect();
            assert!(
                !drawn.iter().any(|s| s == NAMES[1]),
                "at {w}, the audio ring is off the node and still named: {drawn:?}",
            );
            no_name_lands_on_another_layer(w, &shapes, &view);
        }
    }

    /// Every layer on the node is named from [`ALL_NAMED`] points of column up,
    /// which is well under the width a settings column is dragged to. Three
    /// things buy that: the names run along a layer's whole STRETCH rather than
    /// its cell, which is what gives the strip — a few points across at a fresh
    /// view — the empty bar past the stack to be named in; the axis stops a
    /// strip's depth past the quad edge rather than at the billboard's reach,
    /// which is worth a fifth of the bar to the three layers inside it; and a
    /// name whose own stretch cannot hold it borrows the ink-free room beside
    /// it rather than going undrawn.
    ///
    /// The first two reach only a TRAILING layer's empty stretch past the whole
    /// stack, and it is the third that names the audio ring: a twentieth of the
    /// axis at a fresh view is under half what "Audio" wants at any width a
    /// settings column reaches (#405).
    ///
    /// Swept rather than sampled, and the floor is asserted from BOTH sides, so
    /// that a placement change has to move the number here rather than quietly
    /// giving up a name at some width nobody probes. Below the floor the ring's
    /// name is the one that goes — its borrowed room is the first to run out —
    /// and what the bar keeps is a correct three, which is
    /// [`a_name_never_covers_another_layers_cell_more_than_its_own`]'s business.
    #[test]
    fn every_layer_on_the_node_is_named() {
        let named = |w: f32| {
            let mut view = fresh();
            let shapes = shapes(w, |ui| {
                StackBar::new(&mut view).show(ui);
            });
            let drawn: Vec<String> = text_boxes(&shapes).into_iter().map(|(_, s)| s).collect();
            NAMES.iter().filter(|n| drawn.iter().any(|s| s == *n)).count()
        };
        for w in every_column_width().filter(|w| *w >= ALL_NAMED) {
            assert_eq!(named(w), 4, "a layer went unnamed at {w}, past the {ALL_NAMED} floor");
        }
        assert!(
            named(ALL_NAMED - 1.0) < 4,
            "every layer is named a point below the floor too — {ALL_NAMED} is stale, and \
             the bar is now doing better than it claims",
        );
    }

    /// The narrowest settings column at which all four layers are named.
    ///
    /// A measurement of the fresh view's own proportions and the room its four
    /// names need, not a setting: it moves whenever either does.
    const ALL_NAMED: f32 = 193.0;

    /// And one thumb per layer, four of them, on the boundaries.
    #[test]
    fn the_bar_draws_a_thumb_for_every_layer() {
        let mut view = fresh();
        let shapes = shapes(W, |ui| {
            StackBar::new(&mut view).show(ui);
        });
        assert_eq!(handles(&shapes).len(), 4);
    }

    /// The bar this width across, laid out in a real context so a pointer can
    /// be aimed at it, and `frame` a pass of input through it.
    const W: f32 = 400.0;

    /// Where a value on the axis falls, as a fraction of the bar's own width —
    /// what the gesture harness below takes its two ends in.
    fn axis(v: f32) -> f32 {
        (HANDLE_INSET + (W - 2.0 * HANDLE_INSET) * v / AXIS_TOP) / W
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
                |ui| bar.set(StackBar::new(view).show(ui).rect),
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
        assert_eq!(after.ring_inner, before.ring_inner, "the node's middle moved");
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

    /// Letting go forgets what the drag had hold of, so the next press decides
    /// for itself. egui's temp store has no expiry, and a grab left behind is
    /// inherited whole: the second gesture would go on moving the first one's
    /// layer, from wherever on the bar it was aimed.
    #[test]
    fn a_second_gesture_on_the_bar_chooses_for_itself() {
        let rings = fresh().rings();
        let band = axis((rings.band.0 + rings.band.1) * 0.5);
        let middle = axis(rings.inner * 0.5);
        let mut view = fresh();
        gesture(&mut view, |bar| {
            let at = |x: f32| egui::pos2(bar.left() + bar.width() * x, bar.center().y);
            let step = 12.0 / bar.width();
            vec![
                // A drag on the band, and this time it is let go of.
                vec![egui::Event::PointerMoved(at(band))],
                vec![egui::Event::PointerMoved(at(band)), press(at(band), true)],
                vec![egui::Event::PointerMoved(at(band + step))],
                vec![egui::Event::PointerMoved(at(band + 0.05))],
                vec![press(at(band + 0.05), false)],
                // Then one aimed at the node's middle, which is a different
                // stretch of the bar.
                vec![egui::Event::PointerMoved(at(middle))],
                vec![egui::Event::PointerMoved(at(middle)), press(at(middle), true)],
                vec![egui::Event::PointerMoved(at(middle - step))],
                vec![egui::Event::PointerMoved(at(middle - 0.04))],
            ]
        });
        assert!(
            view.band_width > fresh().band_width,
            "the first gesture did not widen the band: {}",
            view.band_width,
        );
        assert!(
            view.ring_inner < fresh().ring_inner,
            "the second gesture did not reach the middle, so the first one's grab outlived it: {}",
            view.ring_inner,
        );
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
