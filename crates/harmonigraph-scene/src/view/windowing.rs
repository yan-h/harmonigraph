//! The windows a [`ViewConfig`] hands out — the block one pane draws
//! ([`ViewConfig::scrolled`]), the naming reach the whole UI shares
//! ([`ViewConfig::reach`]), and the center-following that keeps the picture
//! under the camera.

use super::*;

impl ViewConfig {
    /// The block of lattice one pane's viewport shows: what the camera is
    /// actually looking at, at `aspect`. `derive_scene` is handed the result,
    /// and it is what makes the sheet scroll without end — pan far enough and
    /// the window has walked with you, so there is no edge to reach.
    ///
    /// A [`DrawnWindow`] and not a second [`ViewConfig`], because it is not a
    /// view: it says which nodes to build and nothing about how they look, and
    /// the settings it would otherwise carry a copy of are the ones every pane
    /// shares. The type is also what keeps it from being handed to a reader
    /// that wanted the naming [`reach`](Self::reach); see [`DrawnWindow`] for
    /// the picture that mix-up drew.
    ///
    /// The view's CENTER is not this function's to move: the block
    /// `derive_scene` builds is pinned to the world origin, so a center chosen
    /// here would relabel the pitches without moving the picture. What keeps
    /// the picture under the camera is
    /// [`follow_camera`](Self::follow_camera), which moves the center and the
    /// camera as one. The bounds here are absolute lattice positions about
    /// that center, and they are LOPSIDED wherever the camera's view of the
    /// sheet is — which under perspective is everywhere but dead flat.
    ///
    /// Per DRAW, never written back into the persisted view, and that is the
    /// contract rather than a style: two live copies of the lattice are drawn
    /// every frame — the docked pane and the Video tab's preview — off one
    /// camera at two different aspects, and each must build the window its own
    /// frame shows. A window stored in the shared view would be whichever copy
    /// drew last, leaving the other one drawing for a pane it is not in.
    ///
    /// Aspect, not pixels, is the whole of what it reads, which is what makes
    /// the preview honest: the preview is letterboxed to the render's aspect,
    /// so it derives the same window the mp4 will and shows exactly the nodes
    /// the export gets.
    ///
    /// The sevens window is untouched. How many sheets deep the lattice runs
    /// is a question about the music, not about the pane — the sevens axis has
    /// no screen extent of its own to be read off, only the offset each sheet
    /// is drawn at — so it keeps its bar.
    ///
    /// Falls back to this view's own window where the geometry gives no
    /// rectangle at all (see [`Camera::visible_world_bounds`], which names the
    /// one camera that does that — and it is a degenerate matrix, not a steep
    /// camera). Drawing the reach window there is a picture; an arbitrary huge
    /// number is a stall.
    /// This is the DEMAND — what the geometry asks for, before
    /// [`MAX_DRAWN_NODES`] holds it down.
    /// [`scrolled`](Self::scrolled) is the same window trimmed to fit, and the
    /// two part company exactly where the cap is reached.
    ///
    /// Split out because a test of whether a projection STAYS under the cap
    /// cannot read the trimmed count: `fit_to_node_budget` loops until that
    /// count fits, so it is under the cap by construction and answers about
    /// the trimmer rather than about the projection (#916, which is what
    /// happened).
    pub(crate) fn demanded(&self, camera: &Camera, aspect: f32) -> DrawnWindow {
        let center = self.center();
        let (low, high) = self.sevens_window();
        let flat = |threes: i32, fives: i32| DrawnWindow {
            min: LatticePos::new(center.threes - threes, center.fives - fives, low),
            max: LatticePos::new(center.threes + threes, center.fives + fives, high),
        };
        let spacing = self.spacing;
        // A spacing of zero divides by nothing and a NaN one poisons the
        // rectangle; both leave every step at the same place, where one node
        // is the whole picture there is to draw.
        if spacing.is_nan() || spacing <= 0.0 {
            return flat(0, 0);
        }
        // The slab the sheets occupy, in world depth about the home sheet —
        // `lattice_to_world` puts the sevens axis on z, and the window's
        // center sheet is drawn at the origin. Measured from HOME rather than
        // symmetrically about it: an asymmetric stack leans to one side of the
        // origin, and a slab taken as ±(the deeper end) would ask the camera
        // for a depth with no sheet in it.
        let back = (low - center.sevens) as f32 * spacing;
        let front = (high - center.sevens) as f32 * spacing;
        let Some(sheet) = camera.visible_world_bounds(aspect, back, front) else {
            return self.reach();
        };
        // Where the pane shows the sheets all the way to the horizon there is
        // no far edge to take, and the rectangle's own is a corner's line
        // extrapolated BACKWARDS through the eye — so its two sides are not
        // the two sides of anything. Read as edges they say the picture starts
        // a step or two from the center, which is the foreground missing: at
        // 52° of pitch the rectangle runs from -3.4 to 557 while the pane is
        // showing nodes twelve steps the other way.
        //
        // Mirrored about the center instead, which has no near side to lose,
        // and the node budget rations what is left. That is the answer
        // [`MAX_DRAWN_NODES`] exists to give — there is no window that would
        // be right here — and it costs the lopsided window nothing, because
        // the tilts it wins at are all bounded (cabinet always, the other two
        // out past 45°).
        let (min, max) = if sheet.bounded {
            (sheet.min, sheet.max)
        } else {
            let mirror = sheet.min.abs().max(sheet.max.abs());
            (-mirror, mirror)
        };

        // Margin enough that a node arrives whole and off-pane rather than
        // growing at the edge: its own radius, plus a step for the NAME, which
        // is drawn beside the node and so reaches onto the pane from a node
        // that is not on it.
        let margin = spacing * (1.0 + NODE_RADIUS_FACTOR);
        // Each end of the rectangle taken on its own, in steps from the WORLD
        // ORIGIN — which is where `derive_scene` draws the center node, so a
        // world coordinate divided by the spacing IS an offset from the
        // center, and the block is anchored where the picture is.
        //
        // Taking each end separately is the whole of what keeps the block off
        // the far field. A single extent per axis has to cover the farther end
        // and then mirrors it onto the nearer one, drawing a second copy of
        // the far reach behind the camera that no pane ever shows; see
        // [`DrawnWindow`] for what that costs under perspective.
        //
        // Saturating, so an enormous rectangle lands on the extent bound
        // rather than wrapping. (A NaN casts to 0, which is the center node —
        // also drawable.)
        let offset = |world: f32, round: fn(f32) -> f32| {
            let steps = round(world / spacing);
            let steps = if steps.is_nan() { 0 } else { steps as i32 };
            steps.clamp(-MAX_DRAWN_EXTENT, MAX_DRAWN_EXTENT)
        };
        // World x is the fifths axis and world y the thirds one, which is the
        // one place that mapping has to be undone rather than applied.
        //
        // No center is derived here, and that is deliberate rather than an
        // omission: it would be a relabeling the picture cannot show, and it
        // is not free — under a tilted camera the rectangle's center is a
        // multi-step quantity that moves with every pan and zoom, so rounding
        // it to a step made the pitch drawn at a fixed point on screen JUMP a
        // cell, several times per drag. The center is
        // [`follow_camera`](Self::follow_camera)'s alone, which moves it and
        // the camera together. Only the BOUNDS move here.
        DrawnWindow {
            min: LatticePos::new(
                center.threes + offset(min.y - margin, f32::floor),
                center.fives + offset(min.x - margin, f32::floor),
                low,
            ),
            max: LatticePos::new(
                center.threes + offset(max.y + margin, f32::ceil),
                center.fives + offset(max.x + margin, f32::ceil),
                high,
            ),
        }
    }

    /// The sheets and steps one pane draws, held to
    /// [`MAX_DRAWN_NODES`].
    ///
    /// Every reader wants this one: the trim is what keeps a frame's cost
    /// bounded whatever the camera is asked for. [`demanded`](Self::demanded)
    /// is the window before it, and only a measurement wants that.
    pub fn scrolled(&self, camera: &Camera, aspect: f32) -> DrawnWindow {
        let mut window = self.demanded(camera, aspect);
        window.fit_to_node_budget(self.center());
        window
    }

    /// How far out a played pitch is hunted for a spelling and a node, as a
    /// block — the naming REACH, centered on the camera but sized by the
    /// setting rather than by any pane.
    ///
    /// Not what is drawn; [`scrolled`](Self::scrolled) is. This is one answer
    /// the whole UI shares, so that a name does not change under a pan, and it
    /// is the fallback for the readers that want the picture's window on a
    /// frame where no lattice pane drew one.
    pub fn reach(&self) -> DrawnWindow {
        let center = self.center();
        let extent = LatticePos::new(self.extent_threes.max(0), self.extent_fives.max(0), 0);
        // The sevens ends are absolute sheets rather than a count each side,
        // so they go on whole instead of through the symmetric pair above.
        // They must not be narrowed to a symmetric reach either: this is what
        // names what `scrolled` draws, and a stack leaning one way would have
        // its far sheets drawn out past the block that can spell them.
        let (low, high) = self.sevens_window();
        let min = center - extent;
        let max = center + extent;
        DrawnWindow {
            min: LatticePos::new(min.threes, min.fives, low),
            max: LatticePos::new(max.threes, max.fives, high),
        }
    }

    /// The sheets the picture draws, low end first.
    ///
    /// The one place the pair's order is repaired for a reader, so no window
    /// builder has to: [`sanitize`](Self::sanitize) already orders what it
    /// loads, and this covers the views that never went through it — a test
    /// fixture, and a `ViewConfig` assembled field by field.
    pub fn sevens_window(&self) -> (i32, i32) {
        (self.min_sevens.min(self.max_sevens), self.max_sevens.max(self.min_sevens))
    }

    /// Keep the window's center under the camera, moving both together so the
    /// picture does not stir: a whole step added to the center subtracts one
    /// spacing from every node's world position, and taking the same off the
    /// camera's target leaves each node exactly where it was on screen.
    ///
    /// This is what puts the picture under the camera at all, and both windows
    /// depend on it. The block `derive_scene` builds is pinned to the world
    /// origin, so the camera has to be brought back to the origin for there to
    /// be anything in front of it; and the reach the note names are chosen out
    /// of (see [`reach`](Self::reach)) is centered here, so it has to follow
    /// or scrolling away would leave every note on screen outside the set that
    /// can name it.
    ///
    /// The x and y axes are carried into the center, which is the pair the
    /// lattice's own sheet runs on. The DEPTH is zeroed instead, and that is
    /// not the same operation wearing a different hat: `pan` moves along the
    /// camera's right and up vectors, which under any projection but cabinet
    /// carry a z component, so dragging sideways walks the eye through the
    /// sheets as well as across them. There is nowhere for that to go — which
    /// sheet is home is [`center_sevens`](Self::center_sevens), a setting with
    /// a bar, not somewhere a sideways drag should arrive — and left to
    /// accumulate it is unbounded: 2500 pan gestures under perspective put the
    /// target 747 spacings off the sheet, with the lattice long gone from the
    /// pane and every frame still deriving twenty thousand nodes for it.
    /// Zeroing it makes a pan mean the same thing under all three
    /// projections, which is a slide ACROSS the sheet.
    ///
    /// So the target is left inside one cell of the origin, on every axis.
    /// That matters beyond the picture: the target is persisted, and it is the
    /// number a scroll without end would otherwise grow forever.
    ///
    /// Once a frame, from the docked lattice — which the offline renderer also
    /// reaches, drawing through the same pane function. It is idempotent on a
    /// target already inside its cell, so the extra call costs a render
    /// nothing and its frames stay reproducible.
    pub fn follow_camera(&mut self, camera: &mut Camera) {
        if self.spacing.is_nan() || self.spacing <= 0.0 || !camera.target.is_finite() {
            return;
        }
        // Bounded well inside `i32`, because the center is added to an extent
        // (`reach`) and that sum must not overflow. Saturating the center
        // instead leaves a target that cannot be reduced — it keeps
        // stepping and the center cannot take the step — so the two stop
        // agreeing and the reach comes out EMPTY, which reads as every note
        // being off the lattice, permanently. Clamping the step keeps them in
        // step: an absurd target walks back a bound's worth per frame and
        // arrives.
        let steps = |world: f32| {
            let steps = world / self.spacing;
            if steps.is_finite() {
                steps.round().clamp(-(MAX_CENTER as f32), MAX_CENTER as f32) as i32
            } else {
                0
            }
        };
        let (fives, threes) = (steps(camera.target.x), steps(camera.target.y));
        // Saturating BEFORE the clamp, not after: a center already at the
        // bound plus a step of the bound is `2^31`, one past what an `i32`
        // holds, so the clamp would be handed a number that had already
        // wrapped negative.
        self.center_fives = self.center_fives.saturating_add(fives).clamp(-MAX_CENTER, MAX_CENTER);
        self.center_threes =
            self.center_threes.saturating_add(threes).clamp(-MAX_CENTER, MAX_CENTER);
        camera.target.x -= fives as f32 * self.spacing;
        camera.target.y -= threes as f32 * self.spacing;
        camera.target.z = 0.0;
    }
}
