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
//! folded is a re-layout nobody asked for, and unfolding cannot undo it —
//! the sibling that grew has no memory of what it was. Unfolding is the same
//! trade run backwards: the pane comes back the width it went away, and the
//! window grows by exactly that. The result reads as a rail down the edge of
//! the window, so [`paint`] draws the rail as the pane's own tab, name and all.
//!
//! Resizing the window is the shell's, not ours: [`Folds::apply`] returns the
//! points to lose or regain and the plugin asks its host for them (see
//! `SharedState::take_window_width_change`). Holding the other panes still
//! while the window moves takes more than the folding split's own fraction —
//! every horizontal split ABOVE it has to hand the change outward rather than
//! absorb it, which is [`reflow`].
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
//! own arrow: nothing here duplicates that bookkeeping, it only reads it. What
//! does need remembering is what the fold overwrote — the fraction the user
//! dialed in, and the width the window gave up for it — which is what [`Folds`]
//! holds, one entry per folded split, persisted with the dock and handed back
//! the moment the pane unfolds.
//!
//! That width is not frozen while the pane is away. Resize the window with a
//! pane folded and every pane on screen gives or takes a share of the change;
//! a pane that came back with the width it had BEFORE all that would have been
//! spared a squeeze its neighbours wore, and would take the difference out of
//! them — which is what "opening a pane made the other two smaller" looks like
//! from the outside. So what a rail is holding follows its sibling (see
//! [`Taken::track`]), and folding, resizing and unfolding compose: the pane
//! comes back beside its neighbour in exactly the share it left it in.

use egui_dock::{DockState, Node, NodeIndex, Surface, SurfaceIndex, Tree};

use crate::panes::Tab;

/// Width of egui_dock's collapse-arrow button (its private
/// `Style::TAB_COLLAPSE_BUTTON_SIZE`), which a rail has to be able to hold or
/// there would be no way to unfold what was folded. Tab bars are taller than
/// this in every style the app uses, so the rail is one tab bar thick and the
/// button fits with room to spare; the number is only needed to repaint the
/// button's own square in [`paint`].
const ARROW_BUTTON: f32 = 24.0;

/// What each sideways-folded split is holding for the pane it folded away: the
/// `fraction` it had before it folded, and the width the window gave up for it.
///
/// Persisted with the dock (see `UiPersist`), so a pane folded when the editor
/// window closed still unfolds to the layout — and the window — it came from.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Folds(Vec<Fold>);

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Fold {
    /// Which split, as indices into the dock. Valid only while the tree keeps
    /// its shape — [`Folds::apply`] drops the entry as soon as the node stops
    /// being a horizontal split with a folded child, which covers re-docking.
    surface: usize,
    node: usize,
    /// What to give back on unfold when [`Fold::taken`] cannot say it in
    /// points: a split INSIDE a bigger fold, which only divides what the fold
    /// above it hands down, and an entry from a blob written before folds
    /// moved the window.
    fraction: f32,
    /// What the window gave up for this fold, for the one split per fold that
    /// moved it. `None` on three kinds of entry: the splits inside that fold
    /// (the outermost one asked for the whole subtree's width, so an inner one
    /// asking again would count it twice), every fold in a floating dock
    /// window (no plugin window of its own to take from), and entries restored
    /// from a blob written before folds moved the window.
    ///
    /// serde(default) is what keeps those blobs loadable, as folds that give
    /// back a fraction and no width — which is what they took.
    #[serde(default)]
    taken: Option<Taken>,
}

/// The width one fold took out of the window.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct Taken {
    /// Which child folded. Recorded because a released fold is no longer
    /// collapsed and the tree can no longer say — and because a split whose
    /// folded child CHANGES sides is holding a width that belongs to a pane
    /// that is open again, which is only visible by comparing the two.
    side: Side,
    /// The width unfolding gives that child back, which is the rail it leaves
    /// plus `shrink` — what the window actually gave up, rather than what the
    /// pane had. The two are the same whenever the window had the room; where
    /// it did not, giving back more than was taken would leave the window
    /// wider than it started, one fold at a time.
    width: f32,
    /// The points the window gave up. Held as well as `width` because "Reset
    /// layout" throws the whole arrangement away, with no rail left anywhere
    /// to measure the difference against.
    shrink: f32,
    /// How wide the pane's sibling was when the two above were last measured,
    /// which is what [`Taken::track`] follows the window by.
    ///
    /// serde(default) reads a blob written before folds tracked the window as
    /// zero, which starts the tracking from the first frame it is seen rather
    /// than from a width measured in some other window.
    #[serde(default)]
    kept: f32,
}

impl Taken {
    /// Follow the window: scale what the rail is holding by however much its
    /// sibling has moved since this last ran.
    ///
    /// The sibling is the proxy because it is the pane the folded one shares
    /// its split with, so the two would have taken a window resize in the same
    /// proportion had both been on screen — and because a fold deadens the
    /// separator between them, so nothing else can move it.
    ///
    /// The whole width scales, not just the part outside the rail: what is
    /// held is the width the pane would have if it were OPEN, and an open pane
    /// takes a resize across all of it. Scaling the remainder instead leaves
    /// the pane coming back a rail's worth ahead of its neighbour, which is a
    /// smaller version of the complaint this exists to answer. What the WINDOW
    /// saves by the fold — the width less the rail standing in for it — falls
    /// out of that, and never goes below nothing.
    fn track(&mut self, kept: f32, span: f32) {
        if kept <= 0.0 || !kept.is_finite() {
            return;
        }
        // A blob from before this tracked anything, or a fold on its way back
        // out of a bigger one, has nothing to measure against: start here.
        if self.kept <= 0.0 {
            self.kept = kept;
            return;
        }
        // Under a point, this is not a resize — it is a window landing on a
        // whole pixel a fraction away from what was asked for. Following that
        // would fold the rounding into what the pane is owed, once per fold,
        // in whichever direction the rounding leans. The baseline is left
        // where it is rather than moved, so a drag made a point at a time
        // still adds up to a resize worth following.
        if (kept - self.kept).abs() < 1.0 {
            return;
        }
        self.width = (self.width * kept / self.kept).max(span);
        self.shrink = self.width - span;
        self.kept = kept;
    }
}

/// Which child of a split is the folded one.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
enum Side {
    Left,
    Right,
}

impl Folds {
    /// Squeeze every sideways-folded pane down to a rail, and give back the
    /// width of every split that is not folded any more.
    ///
    /// Runs BEFORE the dock lays out, because `fraction` is the input layout
    /// reads. Idempotent: re-running it on an unchanged dock writes the same
    /// fractions, which is what keeps the rail a fixed number of POINTS wide
    /// as the window resizes (a fraction alone would grow with it).
    ///
    /// `area` is the width `DockArea` is about to lay the main surface out in
    /// — THIS frame's, from the `Ui` (see [`area_width`]), not the rectangles
    /// left in the tree by the last one. Every width below is derived from it
    /// rather than measured, because a fold that resizes the window makes last
    /// frame's rectangles a scale model of this frame's: measure them and a
    /// rail comes out a rail's worth of the wrong window.
    ///
    /// `floor` is the narrowest the shell will let its window become, in the
    /// same points as `area`. A fold asks for no more than the window can
    /// actually give up, because what it records is what it hands back: ask
    /// past the floor and the pane beside it absorbs the difference, while the
    /// unfold pays out the full amount — leaving the window wider than it
    /// started, once per fold. Zero means no floor at all.
    ///
    /// Returns the points the WINDOW has to gain (negative: lose) for the
    /// panes that are not folding to keep their width — zero on every frame
    /// that neither folds nor unfolds anything, which is nearly all of them.
    /// Idempotence is what makes that safe: a fold asks for its width once, on
    /// the frame the arrow is clicked, and the frames after it re-derive the
    /// same layout without asking again. A host that refuses the resize
    /// outright is therefore not fought over it either — the panes settle back
    /// into the window they have, with the rail still exactly a rail.
    #[must_use]
    pub fn apply(
        &mut self,
        dock: &mut DockState<Tab>,
        style: &egui_dock::Style,
        area: f32,
        floor: f32,
    ) -> f32 {
        let mut resize = 0.0;
        let rail = style.tab_bar.height;
        let separator = style.separator.width;
        // What the window has left to give, spent by the folds below and
        // returned by the unfolds. One budget for the whole dock, because
        // there is one window.
        let mut room = (area - floor).max(0.0);
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
            // A floating dock window is laid out in its own window rather than
            // in the dock area, and its size is not ours to know — so that
            // root is measured, a frame stale, which is all a fold there needs:
            // it moves no window and hands its width to the pane beside it.
            let root = if surface == SurfaceIndex::main() {
                area
            } else {
                tree[NodeIndex::root()].rect().map_or(f32::NAN, |rect| rect.width())
            };
            // What is folded, read from the collapsed flags before anything is
            // moved, so that the two passes below agree about it.
            let holds = holds(tree);

            // Unfold first. An entry gives its width back to the layout it took
            // it from, which is the one still in the tree — run this after the
            // folds and it would be measuring rails the same pass had just
            // written. Outermost entry first, for the same reason one level
            // down: an outer fold hands its subtree a width the entries inside
            // it then divide, and a parent always precedes its children in the
            // tree's array.
            let mut granted = derive(tree, root, separator);
            let mut released = Vec::new();
            self.0.retain(|fold| {
                let stays = fold.surface != surface.0
                    || holds.get(fold.node).is_some_and(|hold| hold.holds(fold));
                if !stays {
                    released.push(fold.clone());
                }
                stays
            });
            released.sort_by_key(|fold| fold.node);
            for fold in released {
                let grow = restore(tree, &mut granted, &fold, separator);
                resize += grow;
                room += grow;
            }

            // Then fold, against the width the window is being asked for
            // rather than the one it still has: an unfold above has already
            // moved the root, and a fold measured against the area of a window
            // that is on its way out would take the pane's share of the wrong
            // one. A parent always comes before its children in the tree's
            // array, so one forward pass can carry that down — and a fold that
            // narrows a split this frame has to tell its children itself, or
            // every level below it would settle a frame late.
            let mut granted = derive(tree, granted[0], separator);
            for index in 0..tree.len() {
                let node = NodeIndex(index);
                let (left, right) = (node.left(), node.right());
                if right.0 >= tree.len() || !tree[node].is_horizontal() {
                    continue;
                }
                // Either one child is folded and hands its width to the other,
                // or this split is inside a fold already and divides what it
                // was given into a rail per pane.
                let (span, side, child) = if holds[index].inside {
                    (rail_span(rail_columns(tree, left), rail, separator), Side::Left, None)
                } else if let Some(side) = holds[index].side {
                    let child = match side {
                        Side::Left => left,
                        Side::Right => right,
                    };
                    (rail_span(rail_columns(tree, child), rail, separator), side, Some(child))
                } else {
                    continue;
                };
                // No width to divide yet — a floating window on its first
                // frame, waiting for a rectangle to measure. A fold also waits
                // until there is room for it: a pane squeezed to a rail has to
                // leave its sibling at least one too, or the click that would
                // undo it lands nowhere. A split already inside a fold is
                // exempt from THAT — it was handed exactly the width its rails
                // need — but not from having a width at all.
                let width = granted[index];
                if !width.is_finite()
                    || !(holds[index].inside || width > span + separator + rail)
                {
                    continue;
                }
                let new = !self.0.iter().any(|fold| fold.is(surface, node));
                // What the window gives up, decided on the one frame this fold
                // is new: the pane's whole width, less the rail that stands in
                // for it, and no more than the window has left to give. Only
                // the split that owns the fold asks — one inside it is dividing
                // width already claimed, and asking again would charge the
                // window for it twice — and only on the main surface, since a
                // fold in a floating dock window has no plugin window to take
                // from and keeps egui_dock's trade of handing the width to the
                // sibling.
                let shrink = child
                    .filter(|_| new && surface == SurfaceIndex::main())
                    .map(|child| granted[child.0])
                    .filter(|pane| *pane > span)
                    .map(|pane| (pane - span).min(room))
                    .filter(|shrink| *shrink > 0.0);
                // The width this split is left with once the window has taken
                // its share — which is what the fold is laid out against, so
                // that the frame it settles on is the first one after the
                // resize rather than the one after that.
                let width = width - shrink.unwrap_or(0.0);
                // A split hands each child the space up to half a separator
                // short of its midpoint, so the midpoint has to sit that much
                // further out for the rails themselves to come out `span` wide.
                let edge = (span + separator * 0.5) / width;
                let fraction = match side {
                    Side::Left => edge,
                    Side::Right => 1.0 - edge,
                };
                let rest = width - span - separator;
                granted[index] = width;
                match side {
                    Side::Left => (granted[left.0], granted[right.0]) = (span, rest),
                    Side::Right => (granted[left.0], granted[right.0]) = (rest, span),
                }
                // A rail is not exempt from what the window does next. Resize
                // it while a pane is folded and every pane on screen gives or
                // takes a share; a pane that comes back with the width it had
                // BEFORE all that has been spared a squeeze the others wore,
                // and takes the difference out of them — which reads, from the
                // outside, as opening a pane making its neighbours smaller.
                //
                // So what the rail is holding tracks its sibling, which is the
                // pane it will share the split with and — the separator being
                // dead while a fold pins it — the only one that can move under
                // it. Fold, resize, unfold then lands where resizing alone
                // would have.
                //
                // Only the fold that owns the window's width tracks it. A
                // split INSIDE a bigger fold has no sibling pane to follow —
                // what sits beside it there is another rail — and the fold
                // above it is already following the window on its behalf. It
                // forgets where it was measured from instead, so that coming
                // back out starts a fresh baseline rather than reading the
                // rail it sat beside as a pane that shrank.
                if !new {
                    if let Some(taken) = self
                        .0
                        .iter_mut()
                        .find(|fold| fold.is(surface, node))
                        .and_then(|fold| fold.taken.as_mut())
                    {
                        if holds[index].inside {
                            taken.kept = 0.0;
                        } else {
                            taken.track(rest, span);
                        }
                    }
                }
                let Node::Horizontal(split) = &mut tree[node] else {
                    continue;
                };
                // First frame of this fold: the fraction still in the split is
                // the user's, and this is the last chance to keep it.
                if new {
                    self.0.push(Fold {
                        surface: surface.0,
                        node: node.0,
                        fraction: split.fraction,
                        taken: shrink
                            .map(|shrink| Taken { side, width: span + shrink, shrink, kept: rest }),
                    });
                }
                split.fraction = fraction;
                // The fraction above narrows this split's folded child; this
                // narrows the split itself, all the way out to the window, so
                // that what the fold took comes off the window instead of
                // going to the pane next door.
                if let Some(shrink) = shrink {
                    resize -= shrink;
                    room -= shrink;
                    reflow(tree, &mut granted, node, -shrink, separator);
                }
            }
        }
        // Entries naming a surface the dock no longer has: nothing left to give
        // a width back to, and — as with a re-docked entry — no way to tell
        // whether the pane they were holding is folded somewhere else now, so
        // they are dropped rather than paid out.
        self.0.retain(|fold| reached.contains(&fold.surface));
        resize
    }

    /// Forget every fold, for a dock that is being replaced wholesale (the
    /// Panel pane's "Reset layout"): the indices would otherwise name splits
    /// in a tree that no longer exists.
    ///
    /// Returns the points the window is owed for the folds being forgotten,
    /// the same as unfolding each of them would. A layout reset that left the
    /// window at the width two folds had squeezed it to would hand back the
    /// full arrangement with nowhere to put it.
    #[must_use]
    pub fn clear(&mut self) -> f32 {
        let owed = self.0.iter().filter_map(|fold| fold.taken).map(|taken| taken.shrink).sum();
        self.0.clear();
        owed
    }

    /// Whether anything is being remembered. Nothing in the draw needs this —
    /// it is how a test says "this dock was replaced, so the fractions that
    /// named its splits are gone too".
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Fold {
    fn is(&self, surface: SurfaceIndex, node: NodeIndex) -> bool {
        self.surface == surface.0 && self.node == node.0
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
}

impl Hold {
    /// Whether `fold` is still the fold it was recorded as.
    ///
    /// Being folded is not enough: a split whose folded child changes SIDES —
    /// one pane opening as the other closes, which egui_dock does in a single
    /// click, since expanding a leaf clears the collapsed flag on every
    /// ancestor — is holding a width that belongs to a pane that is open
    /// again. Recognising that as a release is what pays the opened pane back
    /// and lets the closed one be taken afresh.
    fn holds(&self, fold: &Fold) -> bool {
        match (self.inside, self.side, fold.taken) {
            (true, _, _) => true,
            (false, Some(side), Some(taken)) => taken.side == side,
            (false, Some(_), None) => true,
            (false, None, _) => false,
        }
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
        }
    }
    holds
}

/// Every node's width as the dock is about to lay it out: the root gets what it
/// is given, and each split divides its own between its children.
fn derive(tree: &Tree<Tab>, root: f32, separator: f32) -> Vec<f32> {
    let mut granted = vec![f32::NAN; tree.len()];
    granted[0] = root;
    for index in 0..tree.len() {
        let node = NodeIndex(index);
        let (left, right) = (node.left(), node.right());
        if right.0 >= tree.len() || !tree[node].is_parent() {
            continue;
        }
        (granted[left.0], granted[right.0]) = share(&tree[node], granted[index], separator);
    }
    granted
}

/// The widths a split hands its two children, by the arithmetic the dock uses:
/// stacked panes are each as wide as the split, and a horizontal split's
/// children meet at the fraction, half a separator short on either side.
fn share(node: &Node<Tab>, width: f32, separator: f32) -> (f32, f32) {
    match node {
        Node::Vertical(_) => (width, width),
        Node::Horizontal(split) => (
            width * split.fraction - separator * 0.5,
            width * (1.0 - split.fraction) - separator * 0.5,
        ),
        _ => (f32::NAN, f32::NAN),
    }
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

/// Take `delta` points of width out of (negative) or put them into (positive)
/// the subtree at `node`, by rewriting every horizontal split ABOVE it so that
/// the other side of each keeps the width it has.
///
/// This is what sends a fold's width out to the window rather than to the pane
/// beside it. The folding split divides its own width between a rail and a
/// sibling; without this, that is all that changes, and the sibling swallows
/// everything the fold gave up. With it, each split up the tree narrows by the
/// same points while its other child holds still, until the change reaches the
/// root — where nothing is left to absorb it but the window.
///
/// Fractions, not rectangles: the dock lays out from fractions. Vertical
/// splits need no fraction changed — both their children are as wide as the
/// split, so the change passes through — but every split on the way up is now
/// `delta` wider or narrower, and `granted` is corrected as the walk passes so
/// that a second fold in the same pass divides the width the first one left.
///
/// Best effort, and deliberately so: a split with no width to divide yet stops
/// the walk rather than have a fraction derived from `NaN` written into it.
fn reflow(
    tree: &mut Tree<Tab>,
    granted: &mut [f32],
    node: NodeIndex,
    delta: f32,
    separator: f32,
) {
    let mut child = node;
    while let Some(parent) = child.parent() {
        if parent.0 >= tree.len() || parent.0 >= granted.len() {
            return;
        }
        let (whole, grown) = (granted[parent.0], granted[parent.0] + delta);
        if !whole.is_finite() || grown <= 0.0 {
            return;
        }
        let (left, right) = (parent.left(), parent.right());
        if tree[parent].is_horizontal() {
            // The children's widths are already what they are becoming — the
            // one on the way up carries the change, and the other holds still,
            // which is the whole point — so the fraction that puts the two of
            // them in a split this wide is all that is left to write.
            if !granted[left.0].is_finite() {
                return;
            }
            let fraction = (granted[left.0] + separator * 0.5) / grown;
            if let Node::Horizontal(split) = &mut tree[parent] {
                split.fraction = fraction.clamp(0.0, 1.0);
            }
        } else {
            // Stacked, so the pane above or below is as wide as the split and
            // narrows with it. Correcting it here is what lets a second fold
            // in the same pass divide the width the first one left.
            let other = if child == left { right } else { left };
            if other.0 < granted.len() {
                granted[other.0] += delta;
            }
        }
        granted[parent.0] = grown;
        child = parent;
    }
}

/// Undo one fold: give the pane back the width it had, hold its sibling still,
/// and report the points the window has to grow to cover the difference.
///
/// The mirror of what [`Folds::apply`] does when the fold appears, and the
/// reason [`Taken`] records a width in points rather than trusting the
/// remembered fraction to a window that may have been resized since: a pane
/// comes back the size it went away, whatever the window did in between.
///
/// Entries with nothing taken — the splits inside a bigger fold, the folds in a
/// floating dock window, and blobs written before folds moved the window — get
/// the other trade: the fraction back, and no window movement.
fn restore(tree: &mut Tree<Tab>, granted: &mut [f32], fold: &Fold, separator: f32) -> f32 {
    let node = NodeIndex(fold.node);
    // Re-docked out from under the entry: the node it named is not a fold any
    // more, and the pane it was holding may perfectly well still be folded
    // somewhere else in the tree — where this same pass charges the window for
    // it afresh, at the width it has there. Paying the old width back on top of
    // that would move the window twice for one fold, so an entry that loses its
    // split loses its claim.
    if node.0 >= tree.len() || !tree[node].is_horizontal() {
        return 0.0;
    }
    let Some(taken) = fold.taken else {
        set_fraction(tree, node, fold.fraction);
        return 0.0;
    };
    let child = match taken.side {
        Side::Left => node.left(),
        Side::Right => node.right(),
    };
    // The sibling keeps every point it has; the split grows to hold it beside
    // the pane coming back.
    let whole = granted[node.0];
    let kept = granted.get(child.0).map_or(f32::NAN, |rail| whole - rail - separator);
    let grown = kept + taken.width + separator;
    // Finiteness first, and separately: a width that is not a number fails
    // every comparison below rather than tripping one of them.
    if !kept.is_finite() || !grown.is_finite() || kept <= 0.0 || grown <= 0.0 {
        // Nothing to divide, so the fraction is all there is to go on, and the
        // window is left where it is rather than moved off a width nobody can
        // measure.
        set_fraction(tree, node, fold.fraction);
        return 0.0;
    }
    let before = if taken.side == Side::Left { taken.width } else { kept };
    set_fraction(tree, node, (before + separator * 0.5) / grown);
    // The subtree is as wide as the pane just handed back to it, and the split
    // as wide as both — which is what an entry restored after this one, deeper
    // in the same subtree, goes on to divide.
    granted[child.0] = taken.width;
    granted[node.0] = grown;
    reflow(tree, granted, node, grown - whole, separator);
    grown - whole
}

fn set_fraction(tree: &mut Tree<Tab>, node: NodeIndex, fraction: f32) {
    if let Node::Horizontal(split) = &mut tree[node] {
        split.fraction = fraction.clamp(0.0, 1.0);
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
/// rail's own surface, the folded pane's name up it, and an arrow pointing at
/// the space that pane will take when it comes back.
///
/// Runs AFTER the dock, so it works from this frame's rectangles and paints
/// over the parts of the tab bar it is replacing.
///
/// A rail is drawn as the pane's own TAB — the tab's fill, the tab title's type
/// and color — because that is what it has become: a pane too narrow to hold
/// anything but the tab that names it. The tab bar's darker well stays where it
/// always is, in the collapse button's square and the separator beside the
/// rail, so a rail still ends where a pane's edge would.
pub fn paint(ui: &egui::Ui, dock: &DockState<Tab>, style: &egui_dock::Style) {
    let rail = style.tab_bar.height;
    // Frameless mode hides every tab bar, which takes the arrow with it: a
    // fold there is a pane squeezed to nothing, with no chrome to draw.
    if rail <= 0.0 {
        return;
    }
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
            for leaf in leaves(tree, folded) {
                let mut body = leaf.rect;
                body.min.y += rail;
                if body.is_positive() {
                    let fill = style.tab.active.bg_fill;
                    ui.painter().rect_filled(body, egui::CornerRadius::ZERO, fill);
                    if let Some(tab) = leaf.tabs.get(leaf.active.0) {
                        paint_name(ui, body, crate::panes::tab_title(tab), style);
                    }
                }
                paint_arrow(ui, leaf.rect, side, style);
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
}

/// Every leaf in the subtree at `node`, in tree order.
fn leaves(tree: &Tree<Tab>, node: NodeIndex) -> Vec<&egui_dock::LeafNode<Tab>> {
    if node.0 >= tree.len() {
        return Vec::new();
    }
    match &tree[node] {
        Node::Leaf(leaf) => vec![leaf],
        Node::Horizontal(_) | Node::Vertical(_) => {
            let mut found = leaves(tree, node.left());
            found.extend(leaves(tree, node.right()));
            found
        }
        Node::Empty => Vec::new(),
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
/// Skipped rather than clipped when the rail is too short for the whole name:
/// half a word up the side of the window says less than the arrow above it
/// already does.
fn paint_name(ui: &egui::Ui, rail: egui::Rect, name: &str, style: &egui_dock::Style) {
    const PAD: f32 = 8.0;
    let painter = ui.painter();
    let galley = painter.layout_no_wrap(
        name.to_owned(),
        egui::TextStyle::Button.resolve(ui.style()),
        style.tab.active.text_color,
    );
    if galley.size().x + 2.0 * PAD > rail.height() {
        return;
    }
    // Rotating a quarter turn anticlockwise maps the galley's own x onto the
    // rail's height (upwards, hence the anchor at the text's far end) and its
    // height onto the rail's width.
    let anchor = egui::pos2(
        rail.center().x - galley.size().y * 0.5,
        rail.center().y + galley.size().x * 0.5,
    );
    painter.add(
        egui::epaint::TextShape::new(anchor, galley, style.tab.active.text_color)
            .with_angle(-std::f32::consts::FRAC_PI_2),
    );
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
fn paint_arrow(ui: &egui::Ui, leaf: egui::Rect, side: Side, style: &egui_dock::Style) {
    let button =
        egui::Rect::from_min_size(leaf.left_top(), egui::vec2(ARROW_BUTTON, style.tab_bar.height));
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
    #[must_use]
    fn frame_within(folds: &mut Folds, dock: &mut DockState<Tab>, width: f32, floor: f32) -> f32 {
        let change = folds.apply(dock, &style(), width, floor);
        lay_out(dock, width);
        (width + change).max(floor)
    }

    /// One frame in a shell whose window can be as narrow as the fold asks,
    /// which is every test that is not about the floor.
    #[must_use]
    fn frame(folds: &mut Folds, dock: &mut DockState<Tab>, width: f32) -> f32 {
        frame_within(folds, dock, width, 0.0)
    }

    /// A click settled: the frame that asks the window for its new width, and
    /// the frame that is laid out in it.
    #[must_use]
    fn settle(folds: &mut Folds, dock: &mut DockState<Tab>, width: f32) -> f32 {
        settle_within(folds, dock, width, 0.0)
    }

    #[must_use]
    fn settle_within(
        folds: &mut Folds,
        dock: &mut DockState<Tab>,
        width: f32,
        floor: f32,
    ) -> f32 {
        let asked = frame_within(folds, dock, width, floor);
        frame_within(folds, dock, asked, floor)
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
        lay_out(&mut dock, 1000.0);
        let (pane, sibling, column) =
            (width(&dock, LATTICE), width(&dock, SPECTRAL), width(&dock, SETTINGS));
        collapse(&mut dock, Tab::Lattice, true);
        let window = frame(&mut folds, &mut dock, 1000.0);
        assert!(
            (window - (1000.0 - (pane - 26.0))).abs() < 0.01,
            "the window loses the pane's width, less the rail standing in for it"
        );
        // The frame after, at the size the window was asked for, is where the
        // layout settles — and asks for nothing more.
        let settled = frame(&mut folds, &mut dock, window);
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
            lay_out(&mut dock, size);
            collapse(&mut dock, Tab::Lattice, true);
            let window = frame(&mut folds, &mut dock, size);
            let _ = frame(&mut folds, &mut dock, window);
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
        lay_out(&mut dock, 1000.0);
        collapse(&mut dock, Tab::Lattice, true);
        // Twice, because the fold is re-applied every frame: the second pass
        // must not mistake its own rail fraction for the user's.
        let window = frame(&mut folds, &mut dock, 1000.0);
        let window = frame(&mut folds, &mut dock, window);
        collapse(&mut dock, Tab::Lattice, false);
        let window = frame(&mut folds, &mut dock, window);
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
        lay_out(&mut dock, 1000.0);
        let before: Vec<f32> =
            [LATTICE, SPECTRAL, SETTINGS].iter().map(|node| width(&dock, *node)).collect();
        collapse(&mut dock, Tab::Lattice, true);
        let window = frame(&mut folds, &mut dock, 1000.0);
        let window = frame(&mut folds, &mut dock, window);
        collapse(&mut dock, Tab::Lattice, false);
        let window = frame(&mut folds, &mut dock, window);
        let window = frame(&mut folds, &mut dock, window);
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
        lay_out(&mut dock, 1000.0);
        let share = width(&dock, LATTICE) / width(&dock, SPECTRAL);
        collapse(&mut dock, Tab::Lattice, true);
        let window = settle(&mut folds, &mut dock, 1000.0);
        // The user drags the window border in while the lattice is a rail.
        let dragged = window - 120.0;
        let _ = settle(&mut folds, &mut dock, dragged);
        collapse(&mut dock, Tab::Lattice, false);
        let window = settle(&mut folds, &mut dock, dragged);
        let folded = [LATTICE, SPECTRAL, SETTINGS].map(|node| width(&dock, node));

        // Exactly, because both took the resize in the same proportion: the
        // pane comes back beside its neighbour in the share it left it in.
        // This is the one the complaint is about — a pane that comes back at
        // a width measured in an older, wider window is a pane taking that
        // share out of its neighbour.
        assert!(
            (width(&dock, LATTICE) / width(&dock, SPECTRAL) - share).abs() < 0.001,
            "the lattice and the analyzer keep the share they had",
        );

        // And the whole layout lands within a few points of the same dock,
        // never folded, dragged straight to where this one ended up. Not
        // exactly: while a pane is a rail the split above it divides the
        // window between "rail plus analyzer" and the settings column rather
        // than between three panes, so a resize made then is shared out in
        // slightly different proportions — about 6% of the drag, 7 points of
        // the 120 dragged here, and all of it in the column that was never
        // folded rather than between the two panes that trade with it.
        let mut plain = self::dock();
        lay_out(&mut plain, 1000.0);
        lay_out(&mut plain, window);
        for (node, folded) in [LATTICE, SPECTRAL, SETTINGS].iter().zip(folded) {
            let plain = width(&plain, *node);
            assert!(
                (folded - plain).abs() < 8.0,
                "{node:?} came out {folded} across the fold, {plain} across the resize alone",
            );
        }
    }

    /// A settings column folds away as one rail once everything in it is
    /// collapsed: the stacked leaves fold onto each other's tab bars, so the
    /// column itself is one rail wide — and the pictures beside it are no
    /// wider for it, the window is narrower.
    #[test]
    fn a_column_of_collapsed_panes_folds_as_a_single_rail() {
        let mut dock = dock();
        let mut folds = Folds::default();
        lay_out(&mut dock, 1000.0);
        let (column, pictures) = (width(&dock, SETTINGS), width(&dock, PICTURES));
        collapse(&mut dock, Tab::Tuning, true);
        collapse(&mut dock, Tab::Notes, true);
        let window = frame(&mut folds, &mut dock, 1000.0);
        assert!((window - (1000.0 - (column - 26.0))).abs() < 0.01);
        let window = frame(&mut folds, &mut dock, window);
        assert!((width(&dock, SETTINGS) - 26.0).abs() < 0.01);
        assert!((width(&dock, PICTURES) - pictures).abs() < 0.01);
        // And back: a column is folded on the RIGHT of its split, where the
        // pane coming back is the one whose width the split does NOT count
        // from — get that the wrong way round and the two swap widths.
        collapse(&mut dock, Tab::Tuning, false);
        collapse(&mut dock, Tab::Notes, false);
        let window = settle(&mut folds, &mut dock, window);
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
        lay_out(&mut dock, 1000.0);
        let (pair, column) = (width(&dock, PICTURES), width(&dock, SETTINGS));
        collapse(&mut dock, Tab::Lattice, true);
        collapse(&mut dock, Tab::Spectral, true);
        // One pass, not two: the fold tells the split inside it the width it
        // is about to be given rather than leaving it to read that next frame,
        // so both rails are in the same set of fractions.
        let rails = 26.0 + 4.0 + 26.0;
        let window = frame(&mut folds, &mut dock, 1000.0);
        assert!(
            (window - (1000.0 - (pair - rails))).abs() < 0.01,
            "the window loses the pair, less the two rails it leaves"
        );
        // The inner split divides what the fold hands it; only the fold itself
        // charges the window, or the pair would be paid for twice.
        let settled = frame(&mut folds, &mut dock, window);
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
        let window = frame(&mut Folds::default(), &mut dock, 1000.0);
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
        let window = frame(&mut Folds::default(), &mut dock, 1000.0);
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
        lay_out(&mut dock, 1000.0);
        collapse(&mut dock, Tab::Lattice, true);
        let window = frame(&mut folds, &mut dock, 1000.0);
        assert!((window + folds.clear() - 1000.0).abs() < 0.01);
        assert!(folds.is_empty());
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
        lay_out(&mut dock, 1000.0);
        let (lattice, analyzer) = (width(&dock, LATTICE), width(&dock, SPECTRAL));
        // Fold both pictures, a click at a time: the second collapses the pair
        // itself, so the root folds the whole subtree into two rails.
        collapse(&mut dock, Tab::Lattice, true);
        let window = settle(&mut folds, &mut dock, 1000.0);
        collapse(&mut dock, Tab::Spectral, true);
        let window = settle(&mut folds, &mut dock, window);
        // Open the lattice again. The pair's split is folded on the right now.
        collapse(&mut dock, Tab::Lattice, false);
        let window = settle(&mut folds, &mut dock, window);
        assert!((width(&dock, LATTICE) - lattice).abs() < 0.01, "the lattice comes back whole");
        assert!((width(&dock, SPECTRAL) - 26.0).abs() < 0.01, "the analyzer is the rail now");
        collapse(&mut dock, Tab::Spectral, false);
        let window = settle(&mut folds, &mut dock, window);
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
        lay_out(&mut dock, 1000.0);
        collapse(&mut dock, Tab::Lattice, true);
        let window = settle(&mut folds, &mut dock, 1000.0);
        collapse(&mut dock, Tab::Spectral, true);
        let window = settle(&mut folds, &mut dock, window);
        // Collapsing the settings column too leaves the root with two collapsed
        // children and nothing to hand anything to, so every fold is released
        // at once — the inner one recorded first.
        collapse(&mut dock, Tab::Tuning, true);
        collapse(&mut dock, Tab::Notes, true);
        let window = settle(&mut folds, &mut dock, window);
        assert!((window - 1000.0).abs() < 0.01, "both folds hand back what they took");
    }

    /// A window that will not go as narrow as the fold asked for keeps the
    /// difference, and the pane beside the fold absorbs it. What the fold may
    /// NOT do is hand back a width the window never gave up — that leaves the
    /// window wider than it started, one fold at a time.
    #[test]
    fn a_fold_asks_for_no_more_width_than_the_window_can_give() {
        const FLOOR: f32 = 400.0;
        let mut dock = dock();
        let mut folds = Folds::default();
        lay_out(&mut dock, 1000.0);
        collapse(&mut dock, Tab::Lattice, true);
        let window = settle_within(&mut folds, &mut dock, 1000.0, FLOOR);
        // The pair folds as a subtree, and wants more than the floor leaves.
        collapse(&mut dock, Tab::Spectral, true);
        let window = settle_within(&mut folds, &mut dock, window, FLOOR);
        assert!((window - FLOOR).abs() < 0.01, "the window stops at the floor");
        assert!((width(&dock, LATTICE) - 26.0).abs() < 0.01, "the rails are still rails");
        assert!((width(&dock, SPECTRAL) - 26.0).abs() < 0.01);
        // Back out the way we came in, which is the order that never moves the
        // fold from one side of a split to the other.
        collapse(&mut dock, Tab::Spectral, false);
        let window = settle_within(&mut folds, &mut dock, window, FLOOR);
        collapse(&mut dock, Tab::Lattice, false);
        let window = settle_within(&mut folds, &mut dock, window, FLOOR);
        assert!(
            (window - 1000.0).abs() < 0.01,
            "and gives back exactly what it took, not what it asked for"
        );
    }

    /// A pane folded when the editor window closed unfolds, next session, into
    /// the window it took its width out of: the entry goes into the persisted
    /// blob and has to come back knowing what that width was.
    #[test]
    fn a_persisted_fold_still_knows_what_the_window_gave_up() {
        let mut dock = dock();
        let mut folds = Folds::default();
        lay_out(&mut dock, 1000.0);
        let pane = width(&dock, LATTICE);
        collapse(&mut dock, Tab::Lattice, true);
        let window = frame(&mut folds, &mut dock, 1000.0);
        let window = frame(&mut folds, &mut dock, window);
        // Saved and loaded the way `UiPersist` carries it, alongside the dock
        // whose splits the entry names.
        let saved = ron::to_string(&folds).expect("folds serialize");
        let mut folds: Folds = ron::from_str(&saved).expect("folds deserialize");
        collapse(&mut dock, Tab::Lattice, false);
        let window = frame(&mut folds, &mut dock, window);
        let _ = frame(&mut folds, &mut dock, window);
        assert!((window - 1000.0).abs() < 0.01, "the window comes back");
        assert!((width(&dock, LATTICE) - pane).abs() < 0.01, "and the pane with it");
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
            .expect("a pre-`taken` blob still loads");
        lay_out(&mut dock, 1000.0);
        // Nothing in the dock is collapsed, so the entry is released the first
        // time it is looked at — the unfold path, with nothing taken.
        let window = frame(&mut folds, &mut dock, 1000.0);
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
        let window = folds.apply(&mut dock, &style(), 1000.0, 0.0);
        lay_out_surface(&mut dock, floating, 500.0);
        assert_eq!(window, 0.0, "the plugin window is not the one that folded");
        let rail = dock[floating][NodeIndex(1)].rect().expect("on screen").width();
        let sibling = dock[floating][NodeIndex(2)].rect().expect("on screen").width();
        assert!((rail - 26.0).abs() < 0.01, "the fold itself still happens");
        assert!((sibling - (500.0 - 26.0 - 4.0)).abs() < 0.01, "its sibling takes the width");
    }
}
