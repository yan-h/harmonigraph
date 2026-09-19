//! [`DrawnWindow`] — the block of lattice positions a pane builds from, and
//! the node budget it is held to.

use super::*;

/// A block of lattice positions, as explicit inclusive bounds.
///
/// Two of these are in play and they answer different questions: the DRAWN
/// window one pane builds from its camera ([`ViewConfig::scrolled`]), and the
/// naming REACH the whole UI shares ([`ViewConfig::reach`]). A type of their
/// own is what keeps them apart — #357 passed them as two `ViewConfig`s, and
/// its own coverage test then projected through the reach while the renderer
/// drew through the derived window, where the two offsets cancelled and it
/// passed over a picture with sixty-four holes in it.
///
/// Bounds rather than `center ± extent`, because what a camera shows is not
/// centered on anything. Tilt a perspective camera and the sheet runs away
/// from the eye on one side and off the bottom of the pane on the other, so a
/// window forced symmetric about the origin draws a block that is never on
/// screen: at 40° on a 16:9 pane it asks for 25921 nodes where the camera can
/// see 9494. Orthographic and cabinet are symmetric already — their rectangles
/// are square with the view axis — so this axis of the fix is perspective's
/// alone, and it is worth about 2.7x there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawnWindow {
    /// Inclusive, in absolute lattice steps from C — not offsets from a
    /// center, so a reader needs nothing else to know what is in here.
    pub min: LatticePos,
    pub max: LatticePos,
}

impl DrawnWindow {
    /// Every position in the block, threes outer and sevens inner.
    ///
    /// The order is load-bearing: [`index_of`](Self::index_of) inverts it, so
    /// the renderer can find a neighbour by arithmetic instead of hashing.
    pub fn positions(&self) -> impl Iterator<Item = LatticePos> {
        coords::positions_within(
            self.min.threes..=self.max.threes,
            self.min.fives..=self.max.fives,
            self.min.sevens..=self.max.sevens,
        )
    }

    /// How many positions [`positions`](Self::positions) yields, so a
    /// per-frame buffer can preallocate instead of growing through
    /// reallocations.
    pub fn count(&self) -> usize {
        self.span(|p| p.threes) * self.span(|p| p.fives) * self.span(|p| p.sevens)
    }

    /// Where `pos` lands in [`positions`](Self::positions), or `None` when it
    /// is outside the block.
    ///
    /// Each axis is bounds-checked separately, which is the whole of what
    /// keeps an out-of-range step on one axis from aliasing onto a different
    /// node's slot.
    pub fn index_of(&self, pos: LatticePos) -> Option<usize> {
        let axis = |v: i32, lo: i32, hi: i32| (v >= lo && v <= hi).then(|| (v - lo) as usize);
        let t = axis(pos.threes, self.min.threes, self.max.threes)?;
        let f = axis(pos.fives, self.min.fives, self.max.fives)?;
        let s = axis(pos.sevens, self.min.sevens, self.max.sevens)?;
        Some((t * self.span(|p| p.fives) + f) * self.span(|p| p.sevens) + s)
    }

    pub fn contains(&self, pos: LatticePos) -> bool {
        self.index_of(pos).is_some()
    }

    /// One axis's inclusive span, floored at zero so a reversed pair counts as
    /// empty rather than wrapping a `usize`.
    fn span(&self, axis: fn(LatticePos) -> i32) -> usize {
        (axis(self.max) - axis(self.min) + 1).max(0) as usize
    }

    /// Clip the block into [`MAX_DRAWN_NODES`], taking what it loses off the
    /// end of each axis FARTHER from `center` — the horizon — and leaving the
    /// end nearer the center where it is.
    ///
    /// Which end is which is the whole of it, because the block is lopsided
    /// wherever the camera's view of the sheet is. A tilted perspective window
    /// runs a step or two below the eye and hundreds of steps out to the far
    /// field, so a trim that scaled both ends by one factor took the same
    /// FRACTION off each — and a fraction of a near edge already beside the
    /// center is the whole of it. What that drew is a bald wedge across the
    /// lower half of the pane with the lattice running on above it:
    /// `the_budget_trims_the_horizon_not_the_foreground` holds it.
    ///
    /// Toward the CENTER rather than toward the block's own middle.
    /// `follow_camera` keeps the camera's target within one cell of the
    /// center, so the center is what the eye is on; under the tilted camera
    /// that reaches this cap at all, the block's own middle is way out in the
    /// sub-pixel far field.
    ///
    /// Only [`ViewConfig::scrolled`] can reach a block this large, and only
    /// from a camera the picture is already degenerate under — see
    /// [`MAX_DRAWN_NODES`] for what the cap is protecting.
    pub(super) fn fit_to_node_budget(&mut self, center: LatticePos) {
        let count = self.count();
        if count <= MAX_DRAWN_NODES {
            return;
        }
        // Both sheet axes at once, by the square root of how far over budget
        // the count is — the count goes as their product, so that is the
        // factor that lands near the cap in one step whatever shape the block
        // is. The sevens axis is left alone: it is a setting rather than a
        // consequence of the camera, and it is at most nine sheets (see
        // `SEVENS_LAYER_LIMIT`).
        let shrink = (MAX_DRAWN_NODES as f32 / count as f32).sqrt();
        // Spent as a RADIUS about the center that the block is clipped into,
        // rather than as a scale on each bound: an end already inside the
        // radius is left alone, so a lopsided block loses its far side and
        // keeps its near one. On the symmetric block cabinet and orthographic
        // build, where both ends are the same distance out, the two are the
        // same arithmetic — which is why their figures did not move.
        //
        // It is also the closed form of the loop below: take a step off
        // whichever end is farther from the center, over and over, and what
        // you are left with is the block inside a radius.
        let clip = |lo: &mut i32, hi: &mut i32, c: i32| {
            let far = (*hi - c).max(c - *lo).max(0);
            let radius = (far as f32 * shrink) as i32;
            // Neither end is ever clipped PAST the other. A camera tilted to
            // the limit puts the whole visible block to one side of the center
            // (`threes 19..31` at the pitch limit), and clipping its near end
            // across its far one would leave a window with nothing in it.
            let near = (*lo).max(c - radius).min(*hi);
            *hi = (*hi).min(c + radius).max(near);
            *lo = near;
        };
        clip(&mut self.min.threes, &mut self.max.threes, center.threes);
        clip(&mut self.min.fives, &mut self.max.fives, center.fives);
        // The step above lands near the cap rather than exactly on it (each
        // axis carries a `+1` that does not scale, and a lopsided axis keeps
        // its near side whole), so the cap is finished by hand — a step at a
        // time off the longer axis, from whichever of its ends is farther from
        // the center, which is the end with less on it worth seeing.
        while self.count() > MAX_DRAWN_NODES {
            let (lo, hi, c) = if self.span(|p| p.fives) >= self.span(|p| p.threes) {
                (&mut self.min.fives, &mut self.max.fives, center.fives)
            } else {
                (&mut self.min.threes, &mut self.max.threes, center.threes)
            };
            if lo == hi {
                break;
            }
            if c - *lo >= *hi - c {
                *lo += 1;
            } else {
                *hi -= 1;
            }
        }
    }
}
