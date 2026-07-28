//! Folding a pane sideways, so the WINDOW gets the width back.
//!
//! egui_dock's collapse arrow means "hand your space to your sibling": the
//! leaf shrinks to its tab bar and the pane next to it grows into the rest.
//! That is only true inside a VERTICAL split, though — `compute_rect_sizes`
//! special-cases `Node::Vertical` and nothing else — so a leaf in a horizontal
//! split folds its body away and keeps its column, leaving a tall void under
//! the tab bar. In this dock that is most of them: the lattice, the analyzer,
//! and the whole settings column are all children of horizontal splits, which
//! is exactly where collapsing is worth doing.
//!
//! So the horizontal half is done here, on the one lever egui_dock leaves
//! public: the parent split's `fraction`. A folded pane is squeezed to one tab
//! bar's THICKNESS — the same measure the vertical fold uses for its height,
//! and just wide enough for the collapse arrow — and the width it gave up
//! comes off the WINDOW. Every other pane keeps the width it had, which is the
//! difference between folding as a way to make the window narrower and folding
//! as a way to redistribute it: a lattice that doubles because the analyzer
//! folded is a re-layout nobody asked for. The result reads as a rail down the
//! edge of the window, so [`paint`] draws the rail as the pane's own tab, name
//! and all.
//!
//! ## The layout is the fractions; the rails are a rendering of it
//!
//! Nothing here remembers a WIDTH. That is the whole design, and it is the
//! second one: the first remembered what each fold took, in points, and gave
//! it back on the way out — which is right until something else moves. The
//! window resizes and every pane on screen wears a share of it, except the one
//! that is folded, which comes back having been spared and takes the
//! difference out of its neighbours. A pane inside the neighbour folds, and
//! the neighbour narrows for a reason that has nothing to do with the window.
//! A fold the window will not shrink far enough for leaves the difference next
//! door, where measuring it later reads it as the neighbour's own. Each of
//! those is a pane quietly losing width across a fold and an unfold, and every
//! patch for one of them was a new way to get the next wrong.
//!
//! What does not go stale is the fraction the user dialled in. So the fractions
//! ARE the layout, [`Folds`] holds the ones a fold overwrites, and what is on
//! screen is derived from them each frame: the layout as it would be with every
//! pane open, with the folded subtrees squeezed to rails and the difference
//! taken off the window. [`Fit`] is that derivation, and it is linear in one
//! unknown — the window the layout is dialled in AT — which is the only thing
//! carried between frames ([`Dial`]).
//!
//! Everything else falls out of it. A window the user resizes re-dials the
//! layout, folded panes and all, so a fold across a resize lands exactly where
//! the resize alone would have. Folds compose, because two folded subtrees are
//! two terms in the same sum. A resize the window refuses costs nothing, since
//! nothing was booked against it. `every_round_trip_of_clicks_lands_where_it_
//! started` is the test that holds all of that: every sequence of up to six
//! arrow clicks that ends where it started, at three window sizes.
//!
//! Resizing the window is the shell's, not ours: [`Folds::apply`] returns the
//! points to lose or regain and the plugin asks its host for them (see
//! `SharedState::take_window_width_change`).
//!
//! A window that will not go where a fold asked — a host that refused, the
//! floor it will not go under — leaves the layout dialled for a window that is
//! not coming. What is DRAWN there is the layout that fits the window there
//! IS, which is not the dialled one stretched to fill it: a stretch scales
//! every pane by the ratio between the two windows, and a rail is a fixed
//! number of points by construction — at the plugin's 400pt floor that ratio
//! reaches 1.8, which is every rail on screen drawn at 46. The dial itself
//! does not move, so unfolding still hands back exactly what folding took, and
//! the width the window would not give up is spent on the panes still open.
//!
//! The window answers a frame late — the plugin asks its host at the top of
//! the next frame, and egui-baseview reads the window's size for a frame
//! before the UI runs in it — so the two frames after the click are drawn with
//! the settled arrangement stretched across a window that has not shrunk yet.
//! Every pane comes out a factor of `window / (window - fold)` too wide, which
//! for the lattice in a 1000pt editor is 1.85: the picture pair reads at 442
//! points on its way from 698 to 237. That is the whole of the seam, and it is
//! the cost of the pane folding the instant it is clicked — the alternative is
//! to hold the fold back until the window answers, which trades two frames of
//! stretch for two of nothing happening, and leaves the pane unfolded for good
//! if the host refuses.
//!
//! A whole subtree folds the same way, as many rails wide as it has panes that
//! end up beside each other: a collapsed column is one (its panes fold onto
//! each other's tab bars), a collapsed pair is two, and the split inside a
//! folded pair divides the width it was given into one rail each.
//!
//! Whether a pane is folded stays egui_dock's own `collapsed` flag, set by its
//! own arrow: nothing here duplicates that bookkeeping, it only reads it.

use egui_dock::{DockState, Node, NodeIndex, Surface, SurfaceIndex, Tree};

use crate::panes::Tab;

/// Width of egui_dock's collapse-arrow button (its private
/// `Style::TAB_COLLAPSE_BUTTON_SIZE`), which a rail has to be able to hold or
/// there would be no way to unfold what was folded. Tab bars are taller than
/// this in every style the app uses, so the rail is one tab bar thick and the
/// button fits with room to spare; the number is only needed to repaint the
/// button's own square in [`paint`].
const ARROW_BUTTON: f32 = 24.0;

/// The fraction each sideways-folded split was dialled in at, which is the
/// layout: what is on screen is a rendering of it with some panes squeezed to
/// rails, and unfolding is that rendering going away.
///
/// Nothing here records a WIDTH. A width measured when the fold happened is
/// only true until something else moves — the window resized, a pane inside
/// the neighbour folded, a fold the window could not pay for in full leaving
/// the difference next door — and handing a stale one back is how a fold stops
/// being reversible. The fractions are not measurements; they are what the
/// user dialled, and they are as true at one window size as another.
///
/// Persisted with the dock (see `UiPersist`), so a pane folded when the editor
/// window closed still unfolds to the layout it came from.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Folds(Vec<Fold>);

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Fold {
    /// Which split, as indices into the dock. Valid only while the tree keeps
    /// its shape — [`Folds::apply`] drops the entry as soon as the node stops
    /// being folded, which covers re-docking.
    surface: usize,
    node: usize,
    /// The fraction the user dialled in, which the fold overwrites in the tree
    /// and every width below is derived from.
    fraction: f32,
    /// Whether this split was rendering a fold — a rail — when it was last
    /// looked at, as opposed to merely handing a fold below it outward.
    ///
    /// This is what "something folded or unfolded" is read from, and it has to
    /// be per split rather than per entry: a split that already holds a
    /// fraction because a fold sits below it can go on to fold a child of its
    /// own, and the window has to hear about it.
    ///
    /// Runtime-only: a layout is loaded into whatever window it finds, so the
    /// first frame after a load moves nothing.
    #[serde(skip)]
    rail: bool,
}

/// Which child of a split is the folded one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Left,
    Right,
}

impl Folds {
    /// Squeeze every sideways-folded pane down to a rail, and take what it gave
    /// up out of the window.
    ///
    /// Runs BEFORE the dock lays out, because `fraction` is the input layout
    /// reads. Idempotent: re-running it on an unchanged dock writes the same
    /// fractions, which is what keeps a rail a fixed number of POINTS wide as
    /// the window resizes (a fraction alone would grow with it).
    ///
    /// `area` is the width `DockArea` is about to lay the main surface out in
    /// — THIS frame's, from the `Ui` (see [`area_width`]), not the rectangles
    /// left in the tree by the last one, which a resize in flight makes a scale
    /// model of the frame being built.
    ///
    /// `dial` carries the width the user's layout is dialled in AT — the window
    /// it would need with every pane open — and the window last seen. It is the
    /// whole of the bookkeeping.
    ///
    /// - Nothing folded or unfolded this frame: it is re-derived from `area`,
    ///   so a window the user resizes carries the layout with it, folded panes
    ///   and all.
    /// - Something did: it is held, and the difference between the window the
    ///   layout now needs and the one it has is what this returns — the points
    ///   the window has to gain (negative: lose) for every pane that is not
    ///   folding to keep its width.
    ///
    /// `floor` is the narrowest the shell will let the window become. At the
    /// floor the window is no longer a free variable, so the layout stops
    /// following it: without that, a fold the window cannot pay for in full
    /// would re-derive a layout dialled for the window it got rather than the
    /// one it asked for, and unfolding would hand back the difference.
    #[must_use]
    pub fn apply(
        &mut self,
        dock: &mut DockState<Tab>,
        style: &egui_dock::Style,
        area: f32,
        floor: f32,
        dial: &mut Dial,
    ) -> f32 {
        let rail = style.tab_bar.height;
        let separator = style.separator.width;
        let mut ask = 0.0;
        let mut reached = Vec::new();
        for index in 0..dock.surfaces_count() {
            let surface = SurfaceIndex(index);
            let Some(tree) = dock.get_surface_mut(surface).and_then(Surface::node_tree_mut) else {
                continue;
            };
            if tree.is_empty() {
                continue;
            }
            reached.push(surface.0);
            let main = surface == SurfaceIndex::main();
            // A floating dock window is laid out in its own window rather than
            // in the dock area, and its size is not ours to know — so that root
            // is measured, a frame stale, which is all a fold there needs: it
            // moves no window, so its layout simply re-derives to fit.
            let area = if main {
                area
            } else {
                tree[NodeIndex::root()].rect().map_or(f32::NAN, |rect| rect.width())
            };
            if !area.is_finite() || area <= 0.0 {
                continue;
            }
            let holds = holds(tree);
            let moved = self.reconcile(tree, surface, &holds);
            // A floating surface has no window to ask, so its layout is simply
            // the one that fits the window it is in, every frame.
            let mut own = Dial::default();
            let dial = if main { &mut *dial } else { &mut own };
            let fit = Fit::of(tree, &holds, self, surface, rail, separator);
            // Re-dialled when the WINDOW has moved, and only then. That is
            // what carries a resize into a folded layout — and, just as
            // important, what keeps this from re-dialling to a window that is
            // on its way out: the frames between asking for a resize and
            // getting one still measure the old width, and re-deriving from
            // that would undo the fold before the window ever answered. Once
            // the window does land where the fold asked, re-dialling is a
            // no-op, which is what makes "has it moved" the whole test.
            // At the floor the window has stopped answering, so the layout
            // stops following it: re-dialling there would bank the difference
            // between what a fold asked for and what it got as though the user
            // had widened the window, and hand it back on the way out.
            let settled = (area - dial.area).abs() < 0.01 && dial.width > 0.0;
            // An ask that went unanswered — a host that refused, or a floor it
            // will not go under — leaves the layout wanting a window that is
            // not coming. Take what there is instead of drawing past the edge.
            // Not at the floor, where "the window did not move" is what the
            // floor MEANS rather than a refusal — re-dialling there is the
            // inflation the floor pin exists to stop.
            let refused = dial.asked && settled && area > floor + 1.0;
            if (!settled && !moved && area > floor + 1.0) || dial.width <= 0.0 || refused {
                if let Some(dialled) = fit.dialled_for(area) {
                    dial.width = dialled;
                }
            }
            dial.asked = moved;
            dial.area = area;
            let Some(widths) = fit.widths(dial.width) else {
                continue;
            };
            // A fold is a two-step, and the step it is NOT is this one. The
            // window is still the one being left — the resize has been asked
            // for and not yet answered — and every arrangement that fits the
            // old window is a lie about where the panes are going. Drawn
            // settled, they stretch by the ratio between the two windows (1.85
            // for a picture pane in a 1000pt editor). Drawn fitted, the folded
            // pane's neighbour swells to take the freed width and gives it
            // back a frame later. Both read as a flicker for the sake of one
            // frame.
            //
            // So this frame draws what it drew last frame: the fractions in
            // the tree are left where they are, and the layout changes on the
            // frame that has the window it was computed for. What a click
            // costs is a frame of nothing happening, which is a frame nobody
            // sees.
            // Only where there IS a window to wait for. A floating dock
            // window never asks for one, so deferring there would be a frame
            // of nothing followed by a frame of nothing.
            //
            // Drawn at the window there IS, which is not always the one the
            // layout is dialled for: a host that refused the resize, or a
            // floor it will not go under. The dialled layout stretched to fit
            // is the wrong picture of it — fractions scale everything by the
            // ratio between the two windows, rails included, and a rail is a
            // fixed number of points by construction. At the plugin's 400pt
            // floor that ratio reaches 1.8, so every rail on screen comes out
            // at 46. Fitting the window instead leaves the rails alone and
            // spends the difference on the panes that are still open, which
            // is the only place it can come from. `dial` itself does not
            // move, so unfolding still hands back exactly what folding took.
            if !(moved && main) {
                let fitted = fit.dialled_for(area).and_then(|dialled| fit.widths(dialled));
                write_fractions(tree, &holds, fitted.as_ref().unwrap_or(&widths), separator);
            }
            // Only a fold or an unfold moves the window. Any other gap between
            // the layout and the window is one the window is not answering for
            // — a host that refused, or a floor it will not go under — and
            // asking again every frame would be an argument, not a request.
            if main && moved {
                // Never past the widest this window has actually been. Fold a
                // pane and drag the window back out, and the layout is dialled
                // for a window bigger still — the visible panes grew, and the
                // folded one's share grew with them — so unfolding asks for a
                // window that can be twice the display. The host grants it,
                // which is how a plugin window ends up wider than the monitor
                // it is on. Shrinking is never capped; only the growth is.
                dial.widest = dial.widest.max(area);
                let want = widths.window - area;
                ask += want.min((dial.widest - area).max(0.0));
            }
        }
        // Entries naming a surface the dock no longer has.
        self.0.retain(|fold| reached.contains(&fold.surface));
        ask
    }

    /// Take the fraction of every split that has just folded, and give back the
    /// fraction of every split that has just unfolded. Answers whether either
    /// happened, which is what decides if the window has to move.
    fn reconcile(&mut self, tree: &mut Tree<Tab>, surface: SurfaceIndex, holds: &[Hold]) -> bool {
        let mut moved = false;
        self.0.retain(|fold| {
            if fold.surface != surface.0 {
                return true;
            }
            if holds.get(fold.node).is_some_and(Hold::held) {
                return true;
            }
            // Unfolded, or re-docked out from under the entry: the fraction it
            // was holding goes back where it came from.
            if fold.node < tree.len() {
                set_fraction(tree, NodeIndex(fold.node), fold.fraction);
            }
            moved |= fold.rail;
            false
        });
        for (index, hold) in holds.iter().enumerate() {
            let node = NodeIndex(index);
            if !hold.held() {
                continue;
            }
            let rail = hold.folded();
            if let Some(fold) = self.0.iter_mut().find(|fold| fold.is(surface, node)) {
                // A split that was already holding a fraction for a fold below
                // it has now folded a child of its own, or stopped.
                moved |= fold.rail != rail;
                fold.rail = rail;
                continue;
            }
            let Node::Horizontal(split) = &tree[node] else {
                continue;
            };
            // First frame of this fold: the fraction in the split is still the
            // user's, and this is the last chance to keep it.
            self.0.push(Fold { surface: surface.0, node: index, fraction: split.fraction, rail });
            moved |= rail;
        }
        moved
    }

    /// Forget every fold without handing anything back, for a load that brings
    /// its own window along with its layout. The entries name splits by index,
    /// so a tree they were not measured against has to start with none.
    pub fn forget(&mut self) {
        self.0.clear();
    }

    /// Forget every fold, for a dock that is being replaced wholesale (the
    /// Panel pane's "Reset layout"): the indices would otherwise name splits
    /// in a tree that no longer exists.
    ///
    /// Returns the points the window is owed for them. The layout that replaces
    /// this one has every pane open, so it wants the whole width the folds were
    /// keeping off the window.
    #[must_use]
    pub fn clear(&mut self, dial: &Dial, area: f32) -> f32 {
        self.forget();
        // Held to the same ceiling as the rail's arrow (see the ask in
        // [`Folds::apply`]), because it undoes the same fold and therefore owes
        // the same width. A layout dialled for a window that never arrived —
        // a host that refused the fold's resize, or a drag back out while
        // folded — prices itself well above anything the window has been, and
        // a button that hands that price to the host is how the editor ends up
        // wider than the display it is on.
        let want = if dial.width > area { dial.width - area } else { 0.0 };
        want.min((dial.widest - area).max(0.0))
    }

    /// Whether anything is being remembered. Nothing in the draw needs this —
    /// it is how a test says "this dock was replaced, so the fractions that
    /// named its splits are gone too".
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The fraction a split is dialled in at: the one the user set, which for a
    /// folded split is the one held here rather than the rail's in the tree.
    fn dialled(&self, tree: &Tree<Tab>, surface: SurfaceIndex, node: NodeIndex) -> f32 {
        self.0
            .iter()
            .find(|fold| fold.is(surface, node))
            .map(|fold| fold.fraction)
            .unwrap_or_else(|| match &tree[node] {
                Node::Horizontal(split) | Node::Vertical(split) => split.fraction,
                _ => 0.5,
            })
    }
}

impl Fold {
    fn is(&self, surface: SurfaceIndex, node: NodeIndex) -> bool {
        self.surface == surface.0 && self.node == node.0
    }
}

/// What a shell carries between frames for the fold layout: the window width
/// the layout is dialled in at, and the window it last saw.
///
/// Runtime-only. A layout loaded into a window it was not saved at dials itself
/// to the window it finds, which is the same thing that happens when the window
/// is dragged.
#[derive(Default)]
pub struct Dial {
    pub(crate) width: f32,
    area: f32,
    /// Whether the last frame asked the window to change. If the window has
    /// not moved by the next one, the ask was refused and the layout takes
    /// what it has.
    asked: bool,
    /// The widest this window has actually been, which is as far as an unfold
    /// may ask it to grow. See the ask in [`Folds::apply`].
    widest: f32,
}

/// The layout as a function of the window it is dialled in at.
///
/// Every node's width is `slope * dialled + offset` — linear, because a split
/// hands its children a fraction of itself less half a separator, and both of
/// those are constants. That is what lets the window a folded layout needs be
/// SOLVED rather than accumulated: the panes that are folded want
/// `slope * dialled + offset` between them, they are being given a rail each
/// instead, and the difference is what the window does not have to hold.
struct Fit {
    /// Per node, the width it is dialled in at.
    slope: Vec<f32>,
    offset: Vec<f32>,
    /// Per node, the rail span if this split's child is folded away, and which
    /// child it is.
    folds: Vec<Option<(NodeIndex, f32)>>,
    /// Per node, how a rail span divides between its children once this split
    /// is inside a fold: `None` for a stacked split, where both children are
    /// the whole rail.
    columns: Vec<Option<(f32, f32)>>,
}

/// The widths a [`Fit`] comes out at, once the window it is dialled in at is
/// known: what each node gets on screen with the folds rendered as rails, and
/// the window that adds up to.
struct Widths {
    real: Vec<f32>,
    window: f32,
}

impl Fit {
    fn of(
        tree: &Tree<Tab>,
        holds: &[Hold],
        folds: &Folds,
        surface: SurfaceIndex,
        rail: f32,
        separator: f32,
    ) -> Fit {
        let mut fit = Fit {
            slope: vec![0.0; tree.len()],
            offset: vec![0.0; tree.len()],
            folds: vec![None; tree.len()],
            columns: vec![None; tree.len()],
        };
        fit.slope[0] = 1.0;
        for (index, hold) in holds.iter().enumerate() {
            let node = NodeIndex(index);
            let (left, right) = (node.left(), node.right());
            if right.0 >= tree.len() || !tree[node].is_parent() {
                continue;
            }
            // The dialled fractions, not the ones a fold has written into the
            // tree: this is the layout, and the rails are a rendering of it.
            let (slope, offset) = (fit.slope[index], fit.offset[index]);
            match &tree[node] {
                Node::Vertical(_) => {
                    (fit.slope[left.0], fit.offset[left.0]) = (slope, offset);
                    (fit.slope[right.0], fit.offset[right.0]) = (slope, offset);
                }
                _ => {
                    let fraction = folds.dialled(tree, surface, node);
                    (fit.slope[left.0], fit.offset[left.0]) =
                        (slope * fraction, offset * fraction - separator * 0.5);
                    (fit.slope[right.0], fit.offset[right.0]) =
                        (slope * (1.0 - fraction), offset * (1.0 - fraction) - separator * 0.5);
                }
            }
            if tree[node].is_horizontal() {
                fit.columns[index] = Some((
                    rail_span(rail_columns(tree, left), rail, separator),
                    rail_span(rail_columns(tree, right), rail, separator),
                ));
            }
            // Only the outermost fold of a subtree squeezes anything: the
            // splits inside it are dividing a width that is already spoken for.
            if let (Some(side), false) = (hold.side, hold.inside) {
                let child = match side {
                    Side::Left => left,
                    Side::Right => right,
                };
                fit.folds[index] =
                    Some((child, rail_span(rail_columns(tree, child), rail, separator)));
            }
        }
        fit
    }

    /// The window this layout needs so that what is left after the folds is
    /// exactly `area` wide — the inverse of [`Fit::widths`], and the reason
    /// nothing here has to remember a width.
    fn dialled_for(&self, area: f32) -> Option<f32> {
        let mut slope = 1.0;
        let mut offset = 0.0;
        for (child, span) in self.folds.iter().flatten() {
            slope -= self.slope[child.0];
            offset += span - self.offset[child.0];
        }
        // Everything folded: the layout is all rails and no pane, and there is
        // no window that makes it come out at `area`.
        (slope > 0.01).then(|| (area - offset) / slope).filter(|dialled| dialled.is_finite() && *dialled > 0.0)
    }

    /// The widths at a given dialled window: what each node wants, and what it
    /// gets once the folds are rendered as rails.
    fn widths(&self, dialled: f32) -> Option<Widths> {
        if !dialled.is_finite() || dialled <= 0.0 {
            return None;
        }
        let want: Vec<f32> =
            (0..self.slope.len()).map(|i| self.slope[i] * dialled + self.offset[i]).collect();
        // What the folds take out of every split above them, the root included.
        let mut deficit = vec![0.0; want.len()];
        for (index, fold) in self.folds.iter().enumerate() {
            let Some((child, span)) = *fold else { continue };
            // Negative where a pane was dialled narrower than the rail that
            // stands in for it — the window pays those few points rather than
            // saving them, which is what `dialled_for` solves for either way.
            // Skipping them here instead would leave the two disagreeing, and
            // the sibling quietly making up the difference.
            let gap = want[child.0] - span;
            deficit[child.0] += gap;
            let mut node = NodeIndex(index);
            loop {
                deficit[node.0] += gap;
                match node.parent() {
                    Some(parent) => node = parent,
                    None => break,
                }
            }
        }
        let mut real: Vec<f32> = want.iter().zip(&deficit).map(|(w, d)| w - d).collect();
        // Inside a fold, the widths are not the dialled ones divided down —
        // they are the rail span shared out, one rail per pane that ends up
        // beside another. Which is why the deficit above stops at the folded
        // child: everything under it is chrome.
        for (child, span) in self.folds.iter().flatten() {
            self.rails(*child, *span, &mut real);
        }
        Some(Widths { window: real[0], real })
    }

    /// Share a folded subtree's rail span out among the rails it becomes.
    fn rails(&self, node: NodeIndex, span: f32, real: &mut [f32]) {
        if node.0 >= real.len() {
            return;
        }
        real[node.0] = span;
        let (left, right) = (node.left(), node.right());
        if right.0 >= real.len() {
            return;
        }
        match self.columns[node.0] {
            // Stacked panes fold onto each other's tab bars, so both halves of
            // the split are the whole rail.
            None => {
                self.rails(left, span, real);
                self.rails(right, span, real);
            }
            Some((before, after)) => {
                self.rails(left, before, real);
                self.rails(right, after, real);
            }
        }
    }
}

/// Write the fraction every split needs for the widths [`Fit`] worked out.
///
/// Only splits with a fold under them are touched: everywhere else the fraction
/// in the tree IS the dialled one, and rewriting it from a width would only
/// round it off its mark.
fn write_fractions(tree: &mut Tree<Tab>, holds: &[Hold], widths: &Widths, separator: f32) {
    for (index, hold) in holds.iter().enumerate() {
        let node = NodeIndex(index);
        let (left, right) = (node.left(), node.right());
        if right.0 >= tree.len() || !tree[node].is_horizontal() || !hold.held() {
            continue;
        }
        let (whole, before) = (widths.real[index], widths.real[left.0]);
        if !before.is_finite() || whole <= 0.0 {
            continue;
        }
        set_fraction(tree, node, (before + separator * 0.5) / whole);
    }
}

fn set_fraction(tree: &mut Tree<Tab>, node: NodeIndex, fraction: f32) {
    if let Node::Horizontal(split) = &mut tree[node] {
        split.fraction = fraction.clamp(0.0, 1.0);
    }
}

/// What a fold is holding at one split.
#[derive(Clone, Copy, Default)]
struct Hold {
    /// The folded child, when exactly one of the two is collapsed.
    side: Option<Side>,
    /// Inside a fold higher up: this split divides width the fold above it has
    /// already claimed, into a rail per pane, rather than folding on its own
    /// account.
    inside: bool,
    /// A fold sits somewhere BELOW this split, so the fold rewrites its
    /// fraction to send the width outward — which means the fraction in the
    /// tree is no longer the one the user dialled, and the dialled one has to
    /// be held here alongside the folds' own.
    above: bool,
}

impl Hold {
    /// Whether a fold is holding this split at all: either its own child is
    /// folded away, or it sits inside a fold that claimed the whole subtree and
    /// divides what that hands down into a rail per pane.
    fn folded(&self) -> bool {
        self.side.is_some() || self.inside
    }

    /// Whether the fold machinery owns this split's fraction — either because
    /// it is folded, or because a fold below it rewrites the fraction on its
    /// way out to the window. Both have to hand the user's fraction back.
    fn held(&self) -> bool {
        self.folded() || self.above
    }
}

/// Which child of each split a fold is holding, and which splits are inside a
/// fold — from the collapsed flags alone, so nothing here depends on what the
/// same pass has already moved.
fn holds(tree: &Tree<Tab>) -> Vec<Hold> {
    let mut holds = vec![Hold::default(); tree.len()];
    for index in 0..tree.len() {
        let node = NodeIndex(index);
        let (left, right) = (node.left(), node.right());
        if right.0 >= tree.len() || !tree[node].is_parent() {
            continue;
        }
        if holds[index].inside {
            // Everything under a folded subtree folds with it.
            holds[left.0].inside = true;
            holds[right.0].inside = true;
        } else if let Some(side) = folded_side(tree, node) {
            holds[index].side = Some(side);
            holds[match side { Side::Left => left, Side::Right => right }.0].inside = true;
            // Every split above this one hands the width outward, so the fold
            // owns its fraction too.
            let mut above = node;
            while let Some(parent) = above.parent() {
                holds[parent.0].above = true;
                above = parent;
            }
        }
    }
    holds
}



/// Which child of `node` is folded sideways, if exactly one of them is.
///
/// `None` covers the cases where nothing can be handed over: a vertical split
/// (egui_dock folds those itself), neither child collapsed, and both collapsed
/// — which makes the split itself collapsed, so it is its own parent's to fold,
/// as a subtree, into as many rails as it has panes side by side.
fn folded_side(tree: &Tree<Tab>, node: NodeIndex) -> Option<Side> {
    if !tree[node].is_horizontal() {
        return None;
    }
    match (collapsed(tree, node.left()), collapsed(tree, node.right())) {
        (true, false) => Some(Side::Left),
        (false, true) => Some(Side::Right),
        _ => None,
    }
}

fn collapsed(tree: &Tree<Tab>, node: NodeIndex) -> bool {
    node.0 < tree.len() && tree[node].is_collapsed()
}

/// The width `rails` rails need side by side, separators included.
fn rail_span(rails: i32, rail: f32, separator: f32) -> f32 {
    rails.max(1) as f32 * rail + (rails - 1).max(0) as f32 * separator
}

/// How many rails wide `node` comes out once folded: one per leaf that ends up
/// beside another.
///
/// A stack of collapsed leaves is one rail — they fold onto each other's tab
/// bars, top to bottom — which is what lets a whole settings column fold away
/// as a single rail once every pane in it is collapsed. Two collapsed leaves
/// side by side are two rails, and the split between them divides the width
/// they are given into one each.
fn rail_columns(tree: &Tree<Tab>, node: NodeIndex) -> i32 {
    if node.0 >= tree.len() {
        return 0;
    }
    match &tree[node] {
        Node::Leaf(_) => 1,
        Node::Horizontal(_) => {
            rail_columns(tree, node.left()) + rail_columns(tree, node.right())
        }
        Node::Vertical(_) => {
            rail_columns(tree, node.left()).max(rail_columns(tree, node.right()))
        }
        Node::Empty => 0,
    }
}




/// The width `DockArea` is about to lay the main surface out in: what is left
/// of `ui` once the dock has taken its own padding and border out of it.
///
/// From THIS frame's `Ui`, which is the point — a fold that resizes the window
/// makes the rectangles in the tree a scale model of the frame being built,
/// and a rail measured against those is a rail's worth of the wrong window.
pub fn area_width(ui: &egui::Ui, style: &egui_dock::Style) -> f32 {
    let padding = style
        .dock_area_padding
        .map_or(0.0, |margin| f32::from(margin.left) + f32::from(margin.right));
    ui.available_rect_before_wrap().width() - padding - style.main_surface_border_stroke.width
}

/// Draw what a sideways fold needs and egui_dock cannot know it wants: the
/// rail's own surface, and on each pane's stretch of it (see [`name_bands`])
/// the arrow that brings that pane back, with its name under it.
///
/// Runs AFTER the dock, so it works from this frame's rectangles and paints
/// over the parts of the tab bar it is replacing.
///
/// It takes the CLICK too, wherever egui_dock's own button for a pane is not
/// where the pane's stretch of rail begins — down a folded column that is all
/// of them but the first. Hence `&mut`: an arrow that opens nothing would be
/// worse than one in the wrong place, so the button has to move for real
/// rather than just in paint.
///
/// A rail is drawn as the pane's own TAB — the tab's fill, the tab title's type
/// and color — because that is what it has become: a pane too narrow to hold
/// anything but the tab that names it. The tab bar's darker well stays where it
/// always is, in the collapse button's square and the separator beside the
/// rail, so a rail still ends where a pane's edge would.
pub fn paint(ui: &egui::Ui, dock: &mut DockState<Tab>, style: &egui_dock::Style) {
    let rail = style.tab_bar.height;
    // Frameless mode hides every tab bar, which takes the arrow with it: a
    // fold there is a pane squeezed to nothing, with no chrome to draw.
    if rail <= 0.0 {
        return;
    }
    // The pane an arrow of ours was clicked for, applied once the tree is no
    // longer being read from.
    let mut opened = None;
    for index in 0..dock.surfaces_count() {
        let surface = SurfaceIndex(index);
        let Some(tree) = dock.get_surface(surface).and_then(Surface::node_tree) else {
            continue;
        };
        for node in 0..tree.len() {
            let node = NodeIndex(node);
            // A folded side: the rail it left behind, and the arrow that
            // brings it back. Read from the SPLIT rather than from each leaf,
            // because a whole column can fold into one rail — and then the
            // leaves inside it are children of the vertical split, not of the
            // horizontal one that gave the width away.
            let Some(side) = folded_side(tree, node) else {
                continue;
            };
            let folded = match side {
                Side::Left => node.left(),
                Side::Right => node.right(),
            };
            let Some(rect) = tree[folded].rect() else {
                continue;
            };
            // Nothing to draw until the fold has actually been laid out: on
            // the frame a pane is collapsed on it is still its old width, and
            // a rail's worth of chrome across a whole pane would flash.
            let columns = rail_columns(tree, folded);
            if !rect.is_positive()
                || rect.width() >= rail_span(columns, rail, style.separator.width) + rail
            {
                continue;
            }
            for band in name_bands(tree, folded) {
                if paint_band(ui, &band, side, surface, style) {
                    opened = Some((surface, band.node));
                }
            }
            // The separator the rail sits against, and any between rails of a
            // folded pair: all of them inert now, for the same reason.
            let outer = match side {
                Side::Left => rect.right()..=rect.right() + style.separator.width,
                Side::Right => rect.left() - style.separator.width..=rect.left(),
            };
            deaden(ui, egui::Rect::from_x_y_ranges(outer, rect.y_range()), style);
            for band in inner_bands(tree, folded) {
                deaden(ui, band, style);
            }
        }
    }
    if let Some((surface, node)) = opened {
        if let Some(tree) = dock.get_surface_mut(surface).and_then(Surface::node_tree_mut) {
            uncollapse(tree, node);
        }
    }
}

/// One pane's stretch of a rail: its own arrow at the top, its name under it.
/// Answers whether that arrow was clicked.
///
/// The arrow goes at the top of the pane's OWN stretch, which is not where
/// egui_dock puts it. A collapsed leaf gets one tab bar at the top of its
/// rectangle, and down a folded column those rectangles start one after
/// another — so the whole column's arrows end up stacked in the first inches
/// of the rail, identical and all pointing the same way, with nothing to say
/// which pane each one opens. So this draws the arrow where it belongs and
/// takes the click there, and egui_dock's own is painted over and lifted out
/// of the frame's hit test: left live it would open a pane from a point the
/// rail now gives to another pane's name.
fn paint_band(
    ui: &egui::Ui,
    band: &Band,
    side: Side,
    surface: SurfaceIndex,
    style: &egui_dock::Style,
) -> bool {
    if !band.rect.is_positive() {
        return false;
    }
    let rail = style.tab_bar.height;
    ui.painter().rect_filled(band.rect, egui::CornerRadius::ZERO, style.tab.active.bg_fill);
    let arrow = egui::Rect::from_min_size(band.rect.left_top(), egui::vec2(ARROW_BUTTON, rail));
    let mut body = band.rect;
    body.min.y += rail;
    if let Some(tab) = band.leaf.tabs.get(band.leaf.active.0) {
        paint_name(ui, body, crate::panes::tab_title(tab), style);
    }
    paint_arrow(ui, arrow, side, style);
    // Where egui_dock already put the button, a rail with one pane on it and
    // the first pane of a column alike: its own arrow is under ours and needs
    // nothing from us.
    if (band.rect.top() - band.leaf.rect.top()).abs() < 0.5 {
        return false;
    }
    let id = egui::Id::new(("fold arrow", surface.0, band.node.0));
    let clicked = ui.interact(arrow, id, egui::Sense::click()).clicked();
    ui.interact(arrow_button(band.leaf.rect, style), id.with("stacked"), egui::Sense::click());
    clicked
}

/// Open a folded pane, as egui_dock's own arrow would have.
///
/// Its `node_update_collapsed` is crate-private, so the half of it an unfold
/// needs is here: the flag comes off every split above the leaf, since a split
/// is collapsed only while both its children are, and each one's count of
/// collapsed leaves is remade from the two below it — a horizontal split holds
/// as many tab bars as its deeper side, a vertical one as many as both.
///
/// Getting this wrong does not show up as a pane that fails to open; it shows
/// up later, as a collapsed leaf drawn some multiple of a tab bar tall.
fn uncollapse(tree: &mut Tree<Tab>, leaf: NodeIndex) {
    tree[leaf].set_collapsed(false);
    let mut child = leaf;
    while let Some(parent) = child.parent() {
        let (left, right) = (parent.left(), parent.right());
        let (below, beside) =
            (tree[left].collapsed_leaf_count(), tree[right].collapsed_leaf_count());
        tree[parent].set_collapsed(false);
        let leaves = if tree[parent].is_horizontal() { below.max(beside) } else { below + beside };
        tree[parent].set_collapsed_leaf_count(leaves);
        child = parent;
    }
}

/// Each pane in a folded subtree with the stretch of rail that is ITS pane:
/// the arrow that brings it back at the top, its name below that.
///
/// A folded column is ONE rail holding several panes, and egui_dock's division
/// of it is all or nothing: a collapsed leaf is one tab bar tall and whichever
/// is last takes everything left over. So every arrow in the column lands in
/// the first few inches of the rail — one tab bar each, stacked, all pointing
/// the same way — and every name but the last had a 0px body to go in, which
/// is how a folded settings column came to read as "Notes", the pane at the
/// bottom of it.
///
/// The rail is shared out instead by the fractions the column is dialled at,
/// the same ones that decide the panes' heights when it is open. Each pane
/// gets a stretch of rail the size it will come back at, with the button that
/// brings it back at the top of it — the rail as a miniature of the column it
/// restores, rather than a stack of arrows over a list of names.
struct Band<'a> {
    node: NodeIndex,
    leaf: &'a egui_dock::LeafNode<Tab>,
    rect: egui::Rect,
}

fn name_bands(tree: &Tree<Tab>, node: NodeIndex) -> Vec<Band<'_>> {
    let mut bands = Vec::new();
    let Some(rect) = tree[node].rect() else {
        return bands;
    };
    divide(tree, node, rect.top(), rect.bottom(), &mut bands);
    bands
}

/// Share a folded subtree's height out among its panes, as [`Fit::rails`]
/// shares out its width.
fn divide<'a>(
    tree: &'a Tree<Tab>,
    node: NodeIndex,
    top: f32,
    bottom: f32,
    bands: &mut Vec<Band<'a>>,
) {
    if node.0 >= tree.len() {
        return;
    }
    match &tree[node] {
        Node::Leaf(leaf) => bands.push(Band {
            node,
            leaf,
            rect: egui::Rect::from_x_y_ranges(leaf.rect.x_range(), top..=bottom),
        }),
        Node::Vertical(split) => {
            let mid = top + (bottom - top) * split.fraction;
            divide(tree, node.left(), top, mid, bands);
            divide(tree, node.right(), mid, bottom, bands);
        }
        // Panes side by side, a rail each: they divide the fold's WIDTH, so
        // both of them get the whole of its height.
        Node::Horizontal(_) => {
            divide(tree, node.left(), top, bottom, bands);
            divide(tree, node.right(), top, bottom, bands);
        }
        Node::Empty => {}
    }
}

/// Every separator inside the subtree at `node`: the gaps between panes that
/// have folded into rails beside each other.
fn inner_bands(tree: &Tree<Tab>, node: NodeIndex) -> Vec<egui::Rect> {
    if node.0 >= tree.len() || !tree[node].is_parent() {
        return Vec::new();
    }
    let (left, right) = (node.left(), node.right());
    if right.0 >= tree.len() {
        return Vec::new();
    }
    let mut bands = inner_bands(tree, left);
    bands.extend(inner_bands(tree, right));
    if let Node::Horizontal(split) = &tree[node] {
        if let (Some(before), Some(after)) = (tree[left].rect(), tree[right].rect()) {
            bands.push(egui::Rect::from_x_y_ranges(
                before.right()..=after.left(),
                split.rect.y_range(),
            ));
        }
    }
    bands
}

/// Take the grab handle off a separator a fold has pinned.
///
/// egui_dock keeps drawing the separator between a folded pane and its
/// neighbour, hover accent and resize cursor and all, but dragging it can no
/// longer do anything: the fold rewrites the fraction it would set on the very
/// next frame. So the invitation is withdrawn — the same thing egui_dock does
/// for a pane folded downwards, which simply has no separator at all.
fn deaden(ui: &egui::Ui, band: egui::Rect, style: &egui_dock::Style) {
    ui.painter().rect_filled(band, egui::CornerRadius::ZERO, style.separator.color_idle);
    // The cursor is a frame-wide setting rather than a shape, so it is undone
    // by setting it again — which works only because this runs after the dock.
    if ui.rect_contains_pointer(band.expand(style.separator.extra_interact_width * 0.5)) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
    }
}

/// The folded pane's name, read bottom to top up its rail — the one thing a
/// rail can still say about which pane it is, and the difference between a
/// fold and a gap.
///
/// Set as the pane's own tab title would be, in the same type and color, since
/// that is what it stands in for: a rail is the tab of a pane too narrow to
/// hold one, and `TextStyle::Button` is what egui_dock lays a tab title out in.
///
/// Hung from the top of `body`, which is the pane's stretch of rail below its
/// own arrow: the name captions the button that brings the pane back. Centred,
/// it would drift half a window away from that button down a tall rail, and a
/// name that far off says nothing about which arrow it belongs to.
///
/// Skipped rather than clipped when the stretch is too short for the whole
/// name: half a word up the side of the window says less than the arrow above
/// it already does.
fn paint_name(ui: &egui::Ui, body: egui::Rect, name: &str, style: &egui_dock::Style) {
    const PAD: f32 = 8.0;
    let painter = ui.painter();
    let galley = painter.layout_no_wrap(
        name.to_owned(),
        egui::TextStyle::Button.resolve(ui.style()),
        style.tab.active.text_color,
    );
    if !body.is_positive() || galley.size().x + 2.0 * PAD > body.height() {
        return;
    }
    // Rotating a quarter turn anticlockwise maps the galley's own x onto the
    // rail's height (upwards, hence the anchor at the text's far end) and its
    // height onto the rail's width.
    let anchor = egui::pos2(
        body.center().x - galley.size().y * 0.5,
        body.top() + PAD + galley.size().x,
    );
    painter.add(
        egui::epaint::TextShape::new(anchor, galley, style.tab.active.text_color)
            .with_angle(-std::f32::consts::FRAC_PI_2),
    );
}

/// egui_dock's own collapse button for a leaf: its square at the left end of
/// the leaf's tab bar, which for a folded pane is the top of its rect.
fn arrow_button(leaf: egui::Rect, style: &egui_dock::Style) -> egui::Rect {
    egui::Rect::from_min_size(leaf.left_top(), egui::vec2(ARROW_BUTTON, style.tab_bar.height))
}

/// Repaint a rail's collapse arrow to point at the space its pane will take
/// when it comes back: rightwards out of a rail on the left, leftwards out of
/// one on the right.
///
/// Only a rail gets this. An open pane keeps egui_dock's disclosure triangle
/// (down for open, right for collapsed), which claims no direction at all and
/// therefore cannot disagree with the pane next to it — and two open panes DO
/// fold opposite ways, since each shrinks toward its own outer edge, so a
/// direction there is a mismatch on display for no gain: the tab title beside
/// it already says which pane it is. A rail has no title, and which way its
/// pane returns is the only thing left to say.
///
/// The button underneath is left alone and keeps handling the click; this is
/// paint over paint, including the hover fill, which is why it has to run after
/// the dock rather than before it.
fn paint_arrow(ui: &egui::Ui, button: egui::Rect, side: Side, style: &egui_dock::Style) {
    let hovered = ui.rect_contains_pointer(button);
    let painter = ui.painter();
    painter.rect_filled(
        button,
        egui::CornerRadius::ZERO,
        if hovered { style.buttons.collapse_tabs_bg_fill } else { style.tab_bar.bg_fill },
    );
    let color = if hovered {
        style.buttons.collapse_tabs_active_color
    } else {
        style.buttons.collapse_tabs_color
    };
    // The same glyph size egui_dock uses (its `TAB_COLLAPSE_ARROW_SIZE`), so
    // the sideways arrow is the one the dock would have drawn, turned.
    let arrow = egui::Rect::from_center_size(button.center(), egui::Vec2::splat(10.0));
    painter.add(egui::Shape::convex_polygon(
        if side == Side::Left {
            vec![arrow.left_top(), arrow.right_center(), arrow.left_bottom()]
        } else {
            vec![arrow.right_top(), arrow.left_center(), arrow.right_bottom()]
        },
        color,
        egui::Stroke::NONE,
    ));
    // egui_dock's own right-hand border, which the fill above covered.
    painter.vline(
        button.right(),
        button.y_range(),
        egui::Stroke::new(
            ui.ctx().pixels_per_point().recip(),
            style.buttons.collapse_tabs_border_color,
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME_HEIGHT: f32 = 600.0;

    /// The plugin's own arrangement: the lattice and the analyzer share the
    /// left half, the settings column and the notes are stacked on the right.
    /// Node indices come out as they do in the real dock — 1 is the picture
    /// pair, 2 the settings column, 3 and 4 the pictures themselves.
    fn dock() -> DockState<Tab> {
        let mut dock = DockState::new(vec![Tab::Lattice]);
        let surface = dock.main_surface_mut();
        let [pictures, settings] =
            surface.split_right(NodeIndex::root(), 0.7, vec![Tab::Tuning]);
        surface.split_below(settings, 0.5, vec![Tab::Notes]);
        surface.split_right(pictures, 0.7, vec![Tab::Spectral]);
        dock
    }

    fn style() -> egui_dock::Style {
        let mut style = egui_dock::Style::from_egui(&egui::Style::default());
        style.tab_bar.height = 26.0;
        style.separator.width = 4.0;
        style
    }

    /// What the dock does between two [`Folds::apply`] calls: hand every
    /// split's rectangle down to its children the way `compute_rect_sizes`
    /// does, so a test can watch a fold settle the way a frame would.
    ///
    /// Vertical splits are laid out by their fraction alone, without
    /// egui_dock's collapsed-leaf rule. The fold reads the WIDTH of horizontal
    /// splits and nothing else, so the difference never reaches it.
    fn lay_out(dock: &mut DockState<Tab>, width: f32) {
        lay_out_surface(dock, SurfaceIndex::main(), width);
    }

    fn lay_out_surface(dock: &mut DockState<Tab>, surface: SurfaceIndex, width: f32) {
        let separator = style().separator.width;
        let frame = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, FRAME_HEIGHT));
        let tree = &mut dock[surface];
        tree[NodeIndex::root()].set_rect(frame);
        for index in 0..tree.len() {
            let node = NodeIndex(index);
            let (left, right) = (node.left(), node.right());
            if right.0 >= tree.len() {
                continue;
            }
            let Some(rect) = tree[node].rect() else {
                continue;
            };
            let (before, after) = match &tree[node] {
                Node::Horizontal(split) => {
                    let mid = rect.left() + rect.width() * split.fraction;
                    (
                        rect.intersect(egui::Rect::everything_left_of(mid - separator * 0.5)),
                        rect.intersect(egui::Rect::everything_right_of(mid + separator * 0.5)),
                    )
                }
                Node::Vertical(split) => {
                    let mid = rect.top() + rect.height() * split.fraction;
                    (
                        rect.intersect(egui::Rect::everything_above(mid - separator * 0.5)),
                        rect.intersect(egui::Rect::everything_below(mid + separator * 0.5)),
                    )
                }
                _ => continue,
            };
            tree[left].set_rect(before);
            tree[right].set_rect(after);
        }
    }

    /// One frame at a window `width`, in a shell that will not take its window
    /// below `floor`: fold, lay the result out, and hand back the width the
    /// window is being asked to become — which is what the next frame is laid
    /// out at, exactly as a shell would.
    ///
    /// A fold therefore takes two frames to settle in a test as it does in the
    /// editor: one that decides what to ask the window for, and one at the
    /// size it was given.
    /// The one number a shell carries between frames for the fold layout (see
    /// `SharedState::dialled_width`). Tests thread it the same way.
    #[must_use]
    fn frame_within(
        folds: &mut Folds,
        dock: &mut DockState<Tab>,
        dial: &mut Dial,
        width: f32,
        floor: f32,
    ) -> f32 {
        let change = folds.apply(dock, &style(), width, floor, dial);
        lay_out(dock, width);
        (width + change).max(floor)
    }

    /// One frame in a shell whose window can be as narrow as the fold asks,
    /// which is every test that is not about the floor.
    #[must_use]
    fn frame(folds: &mut Folds, dock: &mut DockState<Tab>, dial: &mut Dial, width: f32) -> f32 {
        frame_within(folds, dock, dial, width, 0.0)
    }

    /// A click settled: the frame that asks the window for its new width, and
    /// the frame that is laid out in it.
    #[must_use]
    fn settle(folds: &mut Folds, dock: &mut DockState<Tab>, dial: &mut Dial, width: f32) -> f32 {
        settle_within(folds, dock, dial, width, 0.0)
    }

    #[must_use]
    fn settle_within(
        folds: &mut Folds,
        dock: &mut DockState<Tab>,
        dial: &mut Dial,
        width: f32,
        floor: f32,
    ) -> f32 {
        let asked = frame_within(folds, dock, dial, width, floor);
        frame_within(folds, dock, dial, asked, floor)
    }

    fn width(dock: &DockState<Tab>, node: NodeIndex) -> f32 {
        dock[SurfaceIndex::main()][node].rect().expect("node is on screen").width()
    }

    fn fraction(dock: &DockState<Tab>, node: NodeIndex) -> f32 {
        match &dock[SurfaceIndex::main()][node] {
            Node::Horizontal(split) | Node::Vertical(split) => split.fraction,
            _ => panic!("{node:?} is not a split"),
        }
    }

    /// One click on a pane's collapse arrow, ancestors and all.
    ///
    /// egui_dock flips the leaf's own flag and then runs `node_update_collapsed`
    /// over its ancestors, which is crate-private, so this reproduces it: a
    /// split whose children are both collapsed is collapsed too, and expanding
    /// anything clears the flag all the way to the root — which is how one
    /// click can move a fold from one child of a split to the other.
    ///
    /// Worth reproducing rather than setting the flags a test happens to want:
    /// the sequence of states a user can actually reach is the whole question
    /// for anything that remembers what it folded.
    fn collapse(dock: &mut DockState<Tab>, tab: Tab, collapsed: bool) {
        let path = dock.find_tab(&tab).expect("tab is in the dock");
        dock[path.surface][path.node].set_collapsed(collapsed);
        let tree = &mut dock[path.surface];
        let mut child = path.node;
        while let Some(parent) = child.parent() {
            let (left, right) = (parent.left(), parent.right());
            let (below, beside) =
                (tree[left].collapsed_leaf_count(), tree[right].collapsed_leaf_count());
            if !collapsed {
                tree[parent].set_collapsed(false);
            } else if tree[left].is_collapsed() && tree[right].is_collapsed() {
                tree[parent].set_collapsed(true);
            }
            let leaves =
                if tree[parent].is_horizontal() { below.max(beside) } else { below + beside };
            tree[parent].set_collapsed_leaf_count(leaves);
            child = parent;
        }
    }

    /// Node indices, named as the plugin's dock lays them out.
    const PICTURES: NodeIndex = NodeIndex(1);
    const SETTINGS: NodeIndex = NodeIndex(2);
    const LATTICE: NodeIndex = NodeIndex(3);
    const SPECTRAL: NodeIndex = NodeIndex(4);

    /// The whole point: the width a folded pane gives up comes off the WINDOW,
    /// every other pane keeps the width it had, and the rail left behind is a
    /// tab bar thick rather than a fraction of the window.
    #[test]
    fn folding_a_pane_takes_its_width_out_of_the_window() {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        // A settled frame before the click, as the editor always has: the
        // layout is dialled in at the window it is being drawn in.
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        let (pane, sibling, column) =
            (width(&dock, LATTICE), width(&dock, SPECTRAL), width(&dock, SETTINGS));
        collapse(&mut dock, Tab::Lattice, true);
        let window = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        assert!(
            (window - (1000.0 - (pane - 26.0))).abs() < 0.01,
            "the window loses the pane's width, less the rail standing in for it"
        );
        // The frame after, at the size the window was asked for, is where the
        // layout settles — and asks for nothing more.
        let settled = frame(&mut folds, &mut dock, &mut dial, window);
        assert!((settled - window).abs() < 0.01, "one fold, one resize");
        assert!((width(&dock, LATTICE) - 26.0).abs() < 0.01, "a rail, not a column");
        assert!(
            (width(&dock, SPECTRAL) - sibling).abs() < 0.01,
            "the analyzer keeps the width it had"
        );
        assert!(
            (width(&dock, SETTINGS) - column).abs() < 0.01,
            "so does the settings column, a split further up"
        );
    }

    /// The rail is a fixed number of points, so the same fold at another
    /// window size is the same rail — it must not scale with the window.
    #[test]
    fn a_rail_is_the_same_width_at_any_window_size() {
        for size in [600.0, 2400.0] {
            let mut dock = dock();
            let mut folds = Folds::default();
        let mut dial = Dial::default();
            lay_out(&mut dock, size);
            collapse(&mut dock, Tab::Lattice, true);
            let window = frame(&mut folds, &mut dock, &mut dial, size);
            let _ = frame(&mut folds, &mut dock, &mut dial, window);
            assert!(
                (width(&dock, LATTICE) - 26.0).abs() < 0.01,
                "at {size} wide, in the {window} the fold asked for"
            );
        }
    }

    /// Unfolding is the fraction coming back, not a guess at a new one.
    #[test]
    fn unfolding_gives_back_the_fraction_the_user_had() {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        // A settled frame before the click, as the editor always has: the
        // layout is dialled in at the window it is being drawn in.
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        collapse(&mut dock, Tab::Lattice, true);
        // Twice, because the fold is re-applied every frame: the second pass
        // must not mistake its own rail fraction for the user's.
        let window = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        let window = frame(&mut folds, &mut dock, &mut dial, window);
        collapse(&mut dock, Tab::Lattice, false);
        let window = frame(&mut folds, &mut dock, &mut dial, window);
        assert!((fraction(&dock, PICTURES) - 0.7).abs() < 0.001);
        assert!((window - 1000.0).abs() < 0.01, "and the window it came out of");
    }

    /// Unfolding is folding run backwards: the pane comes back the width it
    /// went away, the panes that never moved still have not, and the window
    /// pays the difference exactly as it was paid.
    #[test]
    fn unfolding_gives_the_window_back_what_folding_took() {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        // A settled frame before the click, as the editor always has: the
        // layout is dialled in at the window it is being drawn in.
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        let before: Vec<f32> =
            [LATTICE, SPECTRAL, SETTINGS].iter().map(|node| width(&dock, *node)).collect();
        collapse(&mut dock, Tab::Lattice, true);
        let window = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        let window = frame(&mut folds, &mut dock, &mut dial, window);
        collapse(&mut dock, Tab::Lattice, false);
        let window = frame(&mut folds, &mut dock, &mut dial, window);
        let window = frame(&mut folds, &mut dock, &mut dial, window);
        assert!((window - 1000.0).abs() < 0.01, "the window is the one it started at");
        for (node, was) in [LATTICE, SPECTRAL, SETTINGS].iter().zip(before) {
            assert!((width(&dock, *node) - was).abs() < 0.01, "{node:?} is back to {was}");
        }
    }

    /// Folding, resizing the window, and unfolding compose to the layout that
    /// resizing alone would have reached.
    ///
    /// A rail is not spared what the window does to everything else. Freeze
    /// what it is holding and the pane comes back at a width measured in some
    /// earlier, wider window, taking the difference out of the panes that DID
    /// wear the resize — which is what "opening a pane made the other two
    /// smaller" looks like from the outside.
    #[test]
    fn a_fold_across_a_resize_lands_where_the_resize_alone_would_have() {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        // A settled frame before the click, as the editor always has: the
        // layout is dialled in at the window it is being drawn in.
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        collapse(&mut dock, Tab::Lattice, true);
        let window = settle(&mut folds, &mut dock, &mut dial, 1000.0);
        // The user drags the window border in while the lattice is a rail.
        let dragged = window - 120.0;
        let _ = settle(&mut folds, &mut dock, &mut dial, dragged);
        collapse(&mut dock, Tab::Lattice, false);
        let window = settle(&mut folds, &mut dock, &mut dial, dragged);
        let folded = [LATTICE, SPECTRAL, SETTINGS].map(|node| width(&dock, node));

        // The same dock, never folded, dragged straight to where this one
        // ended up. The layout is derived from the fractions the user dialled
        // and the window it is being drawn in, and a fold changes neither —
        // so carrying one through a resize is the resize, exactly.
        let mut plain = self::dock();
        lay_out(&mut plain, 1000.0);
        lay_out(&mut plain, window);
        for (node, folded) in [LATTICE, SPECTRAL, SETTINGS].iter().zip(folded) {
            let plain = width(&plain, *node);
            assert!(
                (folded - plain).abs() < 0.5,
                "{node:?} came out {folded} across the fold, {plain} across the resize alone",
            );
        }
    }

    /// A pane folded when the editor window closed unfolds, next session, into
    /// the layout it came from: the entry goes into the persisted blob and has
    /// to come back knowing the fraction it is holding.
    #[test]
    fn a_persisted_fold_still_holds_the_fraction_it_was_dialled_at() {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        let pane = width(&dock, LATTICE);
        collapse(&mut dock, Tab::Lattice, true);
        let window = settle(&mut folds, &mut dock, &mut dial, 1000.0);
        // Saved and loaded the way `UiPersist` carries it, alongside the dock
        // whose splits the entry names, into a window the same size.
        let saved = ron::to_string(&folds).expect("folds serialize");
        let mut folds: Folds = ron::from_str(&saved).expect("folds deserialize");
        let reopened = settle(&mut folds, &mut dock, &mut dial, window);
        assert!((reopened - window).abs() < 0.01, "the editor opens where it closed");
        collapse(&mut dock, Tab::Lattice, false);
        let window = settle(&mut folds, &mut dock, &mut dial, reopened);
        assert!((window - 1000.0).abs() < 0.01, "and the window comes back");
        assert!((width(&dock, LATTICE) - pane).abs() < 0.01, "with the pane in it");
    }

    /// An entry from a blob written before folds moved the window has no width
    /// to give back, and taking one out of that window would move a window
    /// that never gave anything up. The fraction is what those entries hold,
    /// and all they hold — which is also the wire format this has to keep
    /// reading, since a blob it cannot parse costs the whole saved layout.
    #[test]
    fn a_fold_from_before_the_window_moved_gives_back_only_its_fraction() {
        let mut dock = dock();
        let mut folds: Folds = ron::from_str("([(surface:0,node:1,fraction:0.35)])")
            .expect("an older blob still loads");
        let mut dial = Dial::default();
        // A settled frame before the click, as the editor always has: the
        // layout is dialled in at the window it is being drawn in.
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        // Nothing in the dock is collapsed, so the entry is released the first
        // time it is looked at — the unfold path, with nothing taken.
        let window = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        assert!((fraction(&dock, PICTURES) - 0.35).abs() < 0.001, "the fraction it remembered");
        assert_eq!(window, 1000.0, "and no width, because it took none");
        assert!(folds.is_empty());
    }

    /// A fold in a floating dock window is that window's own business: there
    /// is no plugin window behind it to take the width from, so it keeps
    /// egui_dock's trade and hands the width to the pane beside it.
    #[test]
    fn a_fold_in_a_floating_window_leaves_the_plugin_window_alone() {
        let mut dock = dock();
        let floating = dock.add_window(vec![Tab::Nodes]);
        dock[floating].split_right(NodeIndex::root(), 0.5, vec![Tab::Scene]);
        lay_out(&mut dock, 1000.0);
        lay_out_surface(&mut dock, floating, 500.0);
        let path = dock.find_tab(&Tab::Nodes).expect("tab is in the floating window");
        dock[path.surface][path.node].set_collapsed(true);
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        let window = folds.apply(&mut dock, &style(), 1000.0, 0.0, &mut dial);
        lay_out_surface(&mut dock, floating, 500.0);
        assert_eq!(window, 0.0, "the plugin window is not the one that folded");
        let rail = dock[floating][NodeIndex(1)].rect().expect("on screen").width();
        assert!((rail - 26.0).abs() < 0.01, "the fold itself still happens");
    }
    /// A settings column folds away as one rail once everything in it is
    /// collapsed: the stacked leaves fold onto each other's tab bars, so the
    /// column itself is one rail wide — and the pictures beside it are no
    /// wider for it, the window is narrower.
    #[test]
    fn a_column_of_collapsed_panes_folds_as_a_single_rail() {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        // A settled frame before the click, as the editor always has: the
        // layout is dialled in at the window it is being drawn in.
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        let (column, pictures) = (width(&dock, SETTINGS), width(&dock, PICTURES));
        collapse(&mut dock, Tab::Tuning, true);
        collapse(&mut dock, Tab::Notes, true);
        let window = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        assert!((window - (1000.0 - (column - 26.0))).abs() < 0.01);
        let window = frame(&mut folds, &mut dock, &mut dial, window);
        assert!((width(&dock, SETTINGS) - 26.0).abs() < 0.01);
        assert!((width(&dock, PICTURES) - pictures).abs() < 0.01);
        // And back: a column is folded on the RIGHT of its split, where the
        // pane coming back is the one whose width the split does NOT count
        // from — get that the wrong way round and the two swap widths.
        collapse(&mut dock, Tab::Tuning, false);
        collapse(&mut dock, Tab::Notes, false);
        let window = settle(&mut folds, &mut dock, &mut dial, window);
        assert!((window - 1000.0).abs() < 0.01, "the window it came out of");
        assert!((width(&dock, SETTINGS) - column).abs() < 0.01, "the column, as it was");
        assert!((width(&dock, PICTURES) - pictures).abs() < 0.01, "the pictures, still");
    }

    /// Both pictures folded is two rails, not one: they sit side by side, so
    /// neither can be unfolded from a rail it shares. The split between them
    /// divides the width they are given into one each, and the window loses
    /// everything they left.
    #[test]
    fn a_folded_pair_becomes_two_rails_side_by_side() {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        // A settled frame before the click, as the editor always has: the
        // layout is dialled in at the window it is being drawn in.
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        let (pair, column) = (width(&dock, PICTURES), width(&dock, SETTINGS));
        collapse(&mut dock, Tab::Lattice, true);
        collapse(&mut dock, Tab::Spectral, true);
        // One pass, not two: the fold tells the split inside it the width it
        // is about to be given rather than leaving it to read that next frame,
        // so both rails are in the same set of fractions.
        let rails = 26.0 + 4.0 + 26.0;
        let window = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        assert!(
            (window - (1000.0 - (pair - rails))).abs() < 0.01,
            "the window loses the pair, less the two rails it leaves"
        );
        // The inner split divides what the fold hands it; only the fold itself
        // charges the window, or the pair would be paid for twice.
        let settled = frame(&mut folds, &mut dock, &mut dial, window);
        assert!((settled - window).abs() < 0.01, "one fold, one resize");
        assert!((width(&dock, LATTICE) - 26.0).abs() < 0.01, "the lattice's own rail");
        assert!((width(&dock, SPECTRAL) - 26.0).abs() < 0.01, "the analyzer's own rail");
        assert!(
            (width(&dock, PICTURES) - rails).abs() < 0.01,
            "two rails and the separator between them"
        );
        assert!((width(&dock, SETTINGS) - column).abs() < 0.01, "the column keeps its width");
    }

    /// Vertical folds are egui_dock's, and it does them by rect, not fraction:
    /// touching the fraction here would move the pane the user's stacked
    /// neighbour sits at. Nor is there a width to take off the window — a pane
    /// folded downwards gives its height to the pane below it.
    #[test]
    fn a_pane_that_folds_downwards_is_left_alone() {
        let mut dock = dock();
        lay_out(&mut dock, 1000.0);
        collapse(&mut dock, Tab::Tuning, true);
        let window = frame(&mut Folds::default(), &mut dock, &mut Dial::default(), 1000.0);
        assert_eq!(fraction(&dock, SETTINGS), 0.5);
        assert_eq!(fraction(&dock, NodeIndex::root()), 0.7, "the column keeps its width");
        assert_eq!(window, 1000.0, "and the window its own");
    }

    /// With every pane in the dock folded there is no one left to hand the
    /// space to, and a root split has no parent to fold it either — so nothing
    /// folds, and the window is not asked to pay for a fold that did not
    /// happen.
    #[test]
    fn a_fold_with_nowhere_to_give_stays_where_it_is() {
        let mut dock = dock();
        lay_out(&mut dock, 1000.0);
        for tab in [Tab::Lattice, Tab::Spectral, Tab::Tuning, Tab::Notes] {
            collapse(&mut dock, tab, true);
        }
        let window = frame(&mut Folds::default(), &mut dock, &mut Dial::default(), 1000.0);
        assert_eq!(fraction(&dock, NodeIndex::root()), 0.7);
        assert_eq!(fraction(&dock, PICTURES), 0.7);
        assert_eq!(window, 1000.0);
    }

    /// "Reset layout" throws the arrangement away with the folds still in it,
    /// and the window it was squeezed into is no use to a layout where every
    /// pane is open again.
    #[test]
    fn a_layout_reset_hands_back_every_fold_it_holds() {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        // A settled frame before the click, as the editor always has: the
        // layout is dialled in at the window it is being drawn in.
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        collapse(&mut dock, Tab::Lattice, true);
        let window = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        assert!((window + folds.clear(&dial, window) - 1000.0).abs() < 0.01);
        assert!(folds.is_empty());
    }

    /// A host that refuses the fold's resize leaves the layout dialled for a
    /// window that is not coming, and the re-dial that copes with it prices the
    /// arrangement at a window far wider than the one on screen. "Reset layout"
    /// turns that price straight into an ask, so the route that undoes a fold
    /// with a button grows the window where the route that undoes it with the
    /// rail's arrow is capped at the widest the window has been.
    ///
    /// Both routes hand back the same fold, so they owe the same width.
    #[test]
    fn a_layout_reset_asks_for_no_more_window_than_an_unfold_would() {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        collapse(&mut dock, Tab::Lattice, true);
        // The fold asks; the host refuses, so the next frame arrives at the
        // width it already had. That is what re-dials the layout upwards.
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        let window = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        assert!((window - 1000.0).abs() < 0.01, "the host refused, so the window did not move");
        let owed = folds.clear(&dial, window);
        assert!(owed < 0.5, "reset asked the host for {} points on top of {window}", owed.round());
    }

    /// One click can move a fold from one child of a split to the other, and
    /// the width has to move with it: expanding a leaf clears the collapsed
    /// flag on every ancestor, so opening one of a folded pair leaves the pair's
    /// split folded on the OTHER side. Read as still-the-same-fold, the entry
    /// pays the wrong pane back — the one that just opened is left at a rail,
    /// and the window keeps the width of a pane that is on screen.
    #[test]
    fn a_fold_that_changes_sides_pays_back_the_pane_that_opened() {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        // A settled frame before the click, as the editor always has: the
        // layout is dialled in at the window it is being drawn in.
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        let (lattice, analyzer) = (width(&dock, LATTICE), width(&dock, SPECTRAL));
        // Fold both pictures, a click at a time: the second collapses the pair
        // itself, so the root folds the whole subtree into two rails.
        collapse(&mut dock, Tab::Lattice, true);
        let window = settle(&mut folds, &mut dock, &mut dial, 1000.0);
        collapse(&mut dock, Tab::Spectral, true);
        let window = settle(&mut folds, &mut dock, &mut dial, window);
        // Open the lattice again. The pair's split is folded on the right now.
        collapse(&mut dock, Tab::Lattice, false);
        let window = settle(&mut folds, &mut dock, &mut dial, window);
        assert!((width(&dock, LATTICE) - lattice).abs() < 0.01, "the lattice comes back whole");
        assert!((width(&dock, SPECTRAL) - 26.0).abs() < 0.01, "the analyzer is the rail now");
        collapse(&mut dock, Tab::Spectral, false);
        let window = settle(&mut folds, &mut dock, &mut dial, window);
        assert!((window - 1000.0).abs() < 0.01, "the window is the one it started in");
        assert!((width(&dock, SPECTRAL) - analyzer).abs() < 0.01, "and the analyzer with it");
    }

    /// Two folds released in the same pass, one inside the other: the outer
    /// hands its subtree a width the inner one then divides, so it has to go
    /// first. Restored the other way round, the inner fold's refund inflates
    /// what the outer measures itself against and the outer pays back a
    /// NEGATIVE width, stranding the window a fold too narrow for good.
    #[test]
    fn two_folds_released_at_once_hand_back_what_each_took() {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        // A settled frame before the click, as the editor always has: the
        // layout is dialled in at the window it is being drawn in.
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        collapse(&mut dock, Tab::Lattice, true);
        let window = settle(&mut folds, &mut dock, &mut dial, 1000.0);
        collapse(&mut dock, Tab::Spectral, true);
        let window = settle(&mut folds, &mut dock, &mut dial, window);
        // Collapsing the settings column too leaves the root with two collapsed
        // children and nothing to hand anything to, so every fold is released
        // at once — the inner one recorded first.
        collapse(&mut dock, Tab::Tuning, true);
        collapse(&mut dock, Tab::Notes, true);
        let window = settle(&mut folds, &mut dock, &mut dial, window);
        assert!((window - 1000.0).abs() < 0.01, "both folds hand back what they took");
    }

    /// A window that will not go as narrow as the fold asked for keeps the
    /// difference, and the pane beside the fold absorbs it. What the fold may
    /// NOT do is hand back a width the window never gave up — that leaves the
    /// window wider than it started, one fold at a time.
    #[test]
    fn a_fold_the_window_cannot_pay_for_still_gives_it_all_back() {
        const FLOOR: f32 = 400.0;
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        // A settled frame before the click, as the editor always has: the
        // layout is dialled in at the window it is being drawn in.
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        let before = [LATTICE, SPECTRAL, SETTINGS].map(|node| width(&dock, node));
        collapse(&mut dock, Tab::Lattice, true);
        let window = settle_within(&mut folds, &mut dock, &mut dial, 1000.0, FLOOR);
        // The pair folds as a subtree, and wants more than the floor leaves.
        collapse(&mut dock, Tab::Spectral, true);
        let window = settle_within(&mut folds, &mut dock, &mut dial, window, FLOOR);
        assert!((window - FLOOR).abs() < 0.01, "the window stops at the floor");
        // Rails, to the point: the window is wider than the layout it holds,
        // and the difference is spent on the panes still open rather than
        // shared out by fraction (see
        // `a_rail_is_the_same_width_in_a_window_that_would_not_shrink`).
        assert!((width(&dock, LATTICE) - 26.0).abs() < 0.01, "the rails are still rails");
        assert!((width(&dock, SPECTRAL) - 26.0).abs() < 0.01);
        // The width the window would not give up is squeezed out of the panes
        // still on screen instead, which is the only place left for it.
        assert!(width(&dock, SETTINGS) > before[2], "the column takes what the window would not");
        // Back out again. The layout is still dialled in at the window it was
        // dialled in at — the floor is where it stopped following the window,
        // exactly so that this comes back whole.
        collapse(&mut dock, Tab::Spectral, false);
        let window = settle_within(&mut folds, &mut dock, &mut dial, window, FLOOR);
        collapse(&mut dock, Tab::Lattice, false);
        let window = settle_within(&mut folds, &mut dock, &mut dial, window, FLOOR);
        assert!((window - 1000.0).abs() < 0.01, "the window it started in");
        for (node, was) in [LATTICE, SPECTRAL, SETTINGS].iter().zip(before) {
            assert!((width(&dock, *node) - was).abs() < 0.01, "{node:?} is back to {was}");
        }
    }


    /// A rail is a fixed number of points in a window that would not shrink,
    /// too — the case the rail's whole width lives or dies on.
    ///
    /// The layout is then dialled for a window it cannot have, and drawing it
    /// stretched across the window it HAS scales every pane by the ratio
    /// between the two. That ratio reaches 1.8 at the plugin's own 400pt
    /// floor, which is 26pt rails drawn at 46 — and it lands on every rail on
    /// screen at once, since they all wear the same stretch.
    #[test]
    fn a_rail_is_the_same_width_in_a_window_that_would_not_shrink() {
        // High enough that the fold cannot have what it asks for: folding the
        // lattice out of a 1000pt window wants a window in the 500s.
        const FLOOR: f32 = 700.0;
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        let _ = frame_within(&mut folds, &mut dock, &mut dial, 1000.0, FLOOR);
        let sibling = width(&dock, SPECTRAL);
        collapse(&mut dock, Tab::Lattice, true);
        let window = settle_within(&mut folds, &mut dock, &mut dial, 1000.0, FLOOR);
        assert!((window - FLOOR).abs() < 0.01, "the window stops at the floor");
        assert!(
            (width(&dock, LATTICE) - 26.0).abs() < 0.01,
            "a rail is a rail: {}",
            width(&dock, LATTICE)
        );
        // Where the width the window would not give up goes instead. It has
        // to land somewhere, and the panes that are still on screen are the
        // only ones with anything to say about how wide they are.
        assert!(width(&dock, SPECTRAL) > sibling, "the analyzer takes what the window would not");
    }

    /// Every sequence of collapse clicks that ends where it started leaves the
    /// LAYOUT where it started.
    ///
    /// The property the rest of this module is written to keep, and the only
    /// one that catches what a hand-written case does not: the leaks here all
    /// came from a second fold interacting with a first, in an order nobody
    /// thought to write down. Four panes, up to six clicks, three window sizes
    /// — every arrangement the arrows can reach and come back from. Six
    /// because the settings column takes two clicks to fold, so anything that
    /// pairs it with another pane needs six to get back.
    ///
    /// Driven through [`Window`], not `frame`, because a shell is part of the
    /// loop: it rounds the width to whole points, holds a floor, and answers a
    /// frame late. Two of the leaks this found lived in exactly that gap.
    #[test]
    fn every_round_trip_of_clicks_lands_where_it_started() {
        let tabs = [Tab::Lattice, Tab::Spectral, Tab::Tuning, Tab::Notes];
        let mut drifted = Vec::new();
        for start in [700.0f32, 1000.0, 1512.0] {
            for length in [2usize, 4, 6] {
                for sequence in sequences(&tabs, length) {
                    let mut dock = dock();
                    let mut folds = Folds::default();
                    let mut window = Window::new(start);
                    window.settle(&mut folds, &mut dock);
                    let before = [LATTICE, SPECTRAL, SETTINGS].map(|node| width(&dock, node));
                    for tab in &sequence {
                        let open = !collapsed_tab(&dock, *tab);
                        collapse(&mut dock, *tab, open);
                        window.settle(&mut folds, &mut dock);
                    }
                    if tabs.iter().any(|tab| collapsed_tab(&dock, *tab)) {
                        continue;
                    }
                    let after = [LATTICE, SPECTRAL, SETTINGS].map(|node| width(&dock, node));
                    let worst = before
                        .iter()
                        .zip(after)
                        .fold(0.0f32, |worst, (was, now)| worst.max((now - was).abs()));
                    if worst > 1.0 {
                        drifted.push(format!("{start}: {sequence:?} drifted {worst:.1}pt"));
                    }
                }
            }
        }
        assert!(drifted.is_empty(), "{} of them, worst first:\n{}", drifted.len(), drifted.join("\n"));
    }

    /// Every sequence of `length` clicks over `tabs`.
    fn sequences(tabs: &[Tab], length: usize) -> Vec<Vec<Tab>> {
        let mut sequences = vec![Vec::new()];
        for _ in 0..length {
            sequences = sequences
                .into_iter()
                .flat_map(|sequence| {
                    tabs.iter().map(move |tab| {
                        let mut next = sequence.clone();
                        next.push(*tab);
                        next
                    })
                })
                .collect();
        }
        sequences
    }

    fn collapsed_tab(dock: &DockState<Tab>, tab: Tab) -> bool {
        let path = dock.find_tab(&tab).expect("docked");
        dock[path.surface][path.node].is_collapsed()
    }

    /// A shell, as the plugin's is: it holds a floor, sizes its window in whole
    /// points, and answers an ask at the TOP of the next frame — the plugin
    /// collects both its own resizes and the host's before the frame's input
    /// is built, so the frame after an ask is laid out at the size it asked
    /// for. What it does not grant (the floor) it simply does not grant.
    struct Window {
        size: f32,
        area: f32,
        pending: Option<f32>,
        dial: Dial,
    }

    impl Window {
        const FLOOR: f32 = 400.0;

        fn new(width: f32) -> Self {
            Window { size: width, area: width, pending: None, dial: Dial::default() }
        }

        fn frame(&mut self, folds: &mut Folds, dock: &mut DockState<Tab>) {
            if let Some(want) = self.pending.take() {
                self.size = want;
                self.area = want;
            }
            let area = self.area;
            let change = folds.apply(dock, &style(), area, Self::FLOOR, &mut self.dial);
            lay_out(dock, area);
            if change.abs() >= 0.5 {
                self.pending = Some((self.size + change).round().max(Self::FLOOR));
            }
            self.area = self.size;
        }

        fn settle(&mut self, folds: &mut Folds, dock: &mut DockState<Tab>) {
            for _ in 0..6 {
                self.frame(folds, dock);
            }
        }
    }


    /// A folded subtree divides its rail span by how many rails each side
    /// holds, not evenly: fold three panes that sit side by side and the split
    /// between "two of them" and "one of them" is not down the middle.
    ///
    /// The fixture everywhere else folds two panes, where the two shares are
    /// equal and any division of the span looks right — so this is the only
    /// test that can tell the difference.
    #[test]
    fn a_folded_subtree_gives_each_side_the_rails_it_holds() {
        let mut dock = DockState::new(vec![Tab::Lattice]);
        let surface = dock.main_surface_mut();
        // Three panes across, nested so one side of the picture split holds two
        // of them: [[Lattice | Spectral] | Notes] beside the settings column.
        let [pictures, _] = surface.split_right(NodeIndex::root(), 0.7, vec![Tab::Tuning]);
        let [pair, _] = surface.split_right(pictures, 0.7, vec![Tab::Notes]);
        surface.split_right(pair, 0.5, vec![Tab::Spectral]);
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        for tab in [Tab::Lattice, Tab::Spectral, Tab::Notes] {
            collapse(&mut dock, tab, true);
        }
        let window = settle(&mut folds, &mut dock, &mut dial, 1000.0);
        let _ = settle(&mut folds, &mut dock, &mut dial, window);
        for tab in [Tab::Lattice, Tab::Spectral, Tab::Notes] {
            let path = dock.find_tab(&tab).expect("docked");
            let rail = dock[path.surface][path.node].rect().expect("on screen").width();
            assert!((rail - 26.0).abs() < 0.01, "{tab:?} came out {rail} wide, not a rail");
        }
    }


    /// Folding a pane and then dragging the window back out leaves the layout
    /// dialled for a window bigger still — the visible panes grew and the
    /// folded one's share grew with them. Unfolding must not then ask for a
    /// window the display cannot hold: the host grants whatever is asked, and
    /// a plugin window wider than the monitor is how that ends.
    #[test]
    fn an_unfold_never_asks_past_the_widest_the_window_has_been() {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut window = Window::new(1500.0);
        window.settle(&mut folds, &mut dock);
        collapse(&mut dock, Tab::Lattice, true);
        window.settle(&mut folds, &mut dock);
        assert!(window.size < 1000.0, "the fold shrank the window to {}", window.size);
        // Dragged back out while the pane is a rail.
        window.size = 1500.0;
        window.area = 1500.0;
        window.settle(&mut folds, &mut dock);
        collapse(&mut dock, Tab::Lattice, false);
        window.settle(&mut folds, &mut dock);
        assert!(
            window.size <= 1500.5,
            "unfolding asked for {} against a window that has never been wider than 1500",
            window.size,
        );
    }

}
