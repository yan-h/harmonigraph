//! [`RingStack`] — where each layer of a node lands — and the [`Stack`] the
//! layers are handed out of.

// Nothing here reaches back into the settings struct in CODE — the stack is
// handed its numbers — but its prose links to a dozen of `ViewConfig`'s bars,
// and an intra-doc link has to resolve in this module's own scope. rustc does
// not count one as a use of the import that makes it resolve (rust#83149), so
// the lint has to be told.
#[allow(unused_imports)]
use super::*;

/// Where each layer of a node lands, in quad UV units, read outward from its
/// center — what [`ViewConfig::rings`] turns the four size bars into.
///
/// The bars are WIDTHS, and a ring's inner edge is wherever the last drawn
/// layer ended plus one [`gap`](Self::gap) — or [`inner`](Self::inner), for the
/// innermost layer left on. That is the whole of what stacking
/// buys: widening one layer slides everything outside it out as far as the quad
/// edge, no bar can be dragged behind its neighbour, and a layer dialled to 0
/// hands its slot AND its gap back to the ones around it instead of leaving a
/// hole.
///
/// The quad edge is where sliding stops and DROPPING starts: a ring is the
/// width its bar reads or it is not drawn, so the first layer the stack can no
/// longer fit whole comes out as an empty pair. Widen the audio ring far enough
/// and the node loses the band, ending as that ring alone — rather than wearing
/// a hairline whose bar reads out a size nothing on screen matches (`stacked`).
///
/// An empty pair is also what makes `outer > inner` the one test for "this ring
/// draws": there is nothing else to ask, and no second flag that could
/// disagree with the geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingStack {
    /// Where the innermost layer left on begins, and so the empty middle a node
    /// carries (see [`ViewConfig::ring_inner`]). 0 seats the stack on the
    /// node's own center.
    ///
    /// A radius the stack is READ from rather than one of its layers: nothing
    /// is drawn inside it, so no pair of radii describes it and no `outer >
    /// inner` test asks whether it is there.
    pub inner: f32,
    /// The audio ring's inner and outer radius, or `(0.0, 0.0)` when it is off
    /// (see [`ViewConfig::spectral_ring_width`]).
    pub audio: (f32, f32),
    /// The octave band's inner and outer radius, or `(0.0, 0.0)` when it is off
    /// (see [`ViewConfig::band_width`]).
    pub band: (f32, f32),
    /// The outer edge of the outermost ring DRAWN, and 0 on a node with no
    /// ring at all.
    ///
    /// What the melody/bass marks stand off, and what a node's billboard is
    /// sized on, so neither has to know which of the rings inside it
    /// happened to be the last one on.
    pub outer: f32,
    /// Where the melody/bass mark strip STARTS: a gap out from
    /// [`outer`](Self::outer), or [`inner`](Self::inner) when the stack is
    /// empty and there is nothing to stand off.
    ///
    /// Settled here rather than left to the renderer because that second
    /// clause is [`Stack::take`]'s rule, and a layer deriving its own inner edge
    /// downstream is a layer that does not get it: the marks are the one slot
    /// `stacked` cannot hand out, since they alone may run past the quad edge
    /// into the billboard's margin instead of being refused there.
    pub mark_inner: f32,
    /// How deep the mark strip is from [`mark_inner`](Self::mark_inner) (see
    /// [`ViewConfig::mark_thickness`]); 0 = off. The OUTER edge is the one
    /// thing about the marks still left to the renderer, which eases the strip
    /// off its own billboard edge.
    pub mark_thickness: f32,
    /// The padding between two drawn layers (see [`ViewConfig::ring_gap`]) —
    /// the RADIAL one alone, this being the stack. What separates one octave
    /// sector from the next is [`ViewConfig::octave_gap`], which no radius on
    /// this struct depends on.
    pub gap: f32,
}

impl RingStack {
    /// The four boundaries the stack is laid out on, read outward: where the
    /// audio ring's slot begins, where the octave band's begins, where the
    /// melody/bass strip's begins, and where that strip ENDS.
    ///
    /// Each is the outer limit of what is INSIDE it — the middle two a
    /// [`gap`](Self::gap) past where a layer stopped, the last one flush, there
    /// being no layer after the marks to stand off, and the first one flush
    /// too, the empty middle being no layer to stand off either. So the four
    /// run middle, audio ring, band, marks: one boundary per handle, and moving
    /// one is that handle's own number changing.
    ///
    /// **A slot a layer WOULD take, where the layer is not drawn**, which is
    /// what makes this different from reading the radii above and the reason
    /// it exists: an off layer has no inner edge of its own, so a control that
    /// sized it by its radii would lose the handle the moment it was switched
    /// off and could never switch it back on. Two boundaries landing on one
    /// point is exactly what an off layer looks like here, and the Layers bar
    /// draws them piled up.
    ///
    /// A REFUSED layer comes out the same way, since the cursor did not move
    /// for it either — see [`Stack::take`]. The picture and the bar then agree:
    /// the layer is not on the node and its handle is not out on the axis.
    pub fn edges(&self) -> [f32; 4] {
        let after_audio = if self.audio.1 > self.audio.0 { self.audio.1 } else { 0.0 };
        [
            self.inner,
            slot_start(after_audio, self.inner, self.gap),
            self.mark_inner,
            self.mark_inner + self.mark_thickness,
        ]
    }
}

/// The stack a node's rings are handed out of, innermost first: a start radius
/// and a cursor at the outer edge of the last layer DRAWN.
///
/// A ring draws at exactly the width its bar reads or it is not drawn at all,
/// which is why a slot that does not fit is REFUSED rather than clipped to the
/// room left. Clipping keeps the layer alive at whatever width the stack
/// happened to leave it — a bar reading 0.19 drawing 0.0008, a hairline at the
/// node's rim that no setting asked for and nothing on screen explains.
///
/// The two ways a layer comes back empty are different questions, and only one
/// of them is about the room: a layer at width 0 is switched off by its own
/// BAR and gives its slot up, where a layer REFUSED is holding a width the
/// room cannot seat. What tells them apart is the width the layer still reads,
/// not the empty pair they share — see `resized` in the Layers bar, which is
/// the one caller that has to.
pub(super) struct Stack {
    /// Where the innermost layer DRAWN begins, whichever layer that turns out
    /// to be (see [`ViewConfig::ring_inner`]).
    pub(super) inner: f32,
    pub(super) cursor: f32,
    /// How far out the stack REACHED, a refusal counting as far as the layer
    /// it could not seat would have gone.
    ///
    /// The cursor answers "what does the next layer stand off?", and stops at
    /// the last layer DRAWN so that a layer switched off gives its slot back.
    /// This answers "how far out is the stack spoken for?", which is a
    /// different question the moment a refusal is in play: the room ran out,
    /// nothing outside can be seated, and the slot is spent rather than free.
    /// The mark strip is the one layer that reads this instead of the cursor —
    /// see [`ViewConfig::rings`].
    pub(super) reach: f32,
    /// Set by a refusal, and the reason it is not just a cursor: the two ways
    /// a layer comes back empty are different questions, and only one of them
    /// is about the room.
    ///
    /// A layer at width 0 is switched off by its own BAR and gives its slot
    /// up; the layer outside it closes over the space, which is what the bar's
    /// hover promises. A layer REFUSED is the room itself running out, and
    /// nothing outside it can fit either — so the stack drops from the outside
    /// in, one layer at a time, and a layer that has gone stays gone while the
    /// room keeps shrinking.
    ///
    /// Letting a refusal leave the cursor where it was makes the refused
    /// layer's slot a gift to the one outside it, which reads on screen as the
    /// stack coming apart in no order at all: push the stack's start past the
    /// audio ring's slot and the BAND takes it, so a band that had been gone
    /// for a quarter of the bar's travel reappears, and the ring's own width
    /// bar picks up a second off position at the TOP of its travel, where
    /// dragging it wider is what removes it.
    pub(super) full: bool,
}

impl Stack {
    /// The next layer's slot: `width` thick, a `gap` out from the cursor,
    /// which it then advances to its own outer edge.
    ///
    /// The gap is skipped while nothing has been drawn, where there is nothing
    /// to stand off: the innermost ring left on seats its inner edge on the
    /// stack's own start, rather than opening a hole the size of a padding
    /// around nothing.
    pub(super) fn take(&mut self, gap: f32, width: f32) -> (f32, f32) {
        if width <= 0.0 || self.full {
            return (0.0, 0.0);
        }
        let inner = slot_start(self.cursor, self.inner, gap);
        let outer = inner + width;
        if outer > 1.0 {
            self.full = true;
            // Spent, not free: the reach stands at the node's own edge, which
            // is where the room ran out. The mark strip then stands off that
            // rather than dropping into the slot, and it does so at the SAME
            // radius it had one step earlier, when the layer fitted exactly —
            // so the strip crosses a refusal without moving at all.
            //
            // The edge and not `outer`, which is where the layer would have
            // ended and is unbounded: `mark_inner` is what the shader sizes
            // every node's BILLBOARD on, so a strip seated on a refused width
            // asks for a quad several node radii across, on every node in the
            // window, to draw marks nobody can see.
            self.reach = 1.0;
            return (0.0, 0.0);
        }
        self.cursor = outer;
        self.reach = outer;
        (inner, outer)
    }
}

/// Where the next layer out begins, given the outer edge of the last one
/// DRAWN: a `gap` past it, or `inner` — the stack's own start — when nothing
/// has been drawn yet.
///
/// The second clause is the whole of why this is a function rather than a sum.
/// The innermost ring left on seats on the start with no padding in front of
/// it, there being no layer inside it to stand off — and three places have to
/// agree about that: [`Stack::take`] handing out a slot,
/// [`ViewConfig::rings`] placing the mark strip, and [`RingStack::edges`]
/// saying where a layer that is switched OFF would have started. A second copy
/// of the rule is how a handle comes to sit a gap off a ring that is not there.
pub(super) fn slot_start(cursor: f32, inner: f32, gap: f32) -> f32 {
    if cursor > 0.0 {
        cursor + gap
    } else {
        inner
    }
}
