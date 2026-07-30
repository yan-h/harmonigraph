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
//! ## The separators beside a fold still resize
//!
//! A fold owns the fraction of every split it sends width out through, and it
//! rewrites each of them every frame — so a separator dragged anywhere across a
//! folded layout snapped back one frame after the handle had lit up and the
//! cursor had changed for it. What a drag names is a boundary in what is DRAWN,
//! and what the fold holds is the layout that is drawn from, so [`Folds::absorb`]
//! carries the drag back through the derivation — by running the derivation, over
//! a bisection, rather than by keeping a second inverse of it that has to agree.
//! Everything else follows from that: the drag is a change to the layout, so a
//! folded pane's dialled share of the split grows with the panes it is dragged
//! beside, exactly as it would have if it were open.
//!
//! The separators a fold has PINNED cannot resize what they divide at all: the
//! one the rail sits against, and, where panes have folded side by side, the ones
//! between the rails. A rail is a fixed number of points, and the split that
//! folded it has its fraction rewritten every frame to keep it that way. But a
//! boundary with a pane somewhere to the left of it and a pane somewhere to the
//! right is a resize to everyone looking at it, whatever the tree says is
//! immediately either side, so that is what it does: the drag goes out to the
//! nearest split that divides two panes which can both change width
//! ([`shove_target`], [`nudge`]), and the rails travel with the boundary at their
//! own width. Every separator across a run of rails therefore moves the same two
//! panes, which is the only thing they could all mean.
//!
//! Where there is nothing outward to pass it to — a fold holding the whole of one
//! side of the window, one open pane and the window's own edge — the only width
//! that can move is the folded pane's, out of the window. So the separator the
//! rail sits against offers exactly that ([`grab`]): pull it and the pane comes
//! back at the width it was pulled to, a pane at a time, as its own arrow would
//! have. A band between two rails there has nothing left to offer and is inert
//! ([`deaden`]).
//!
//! Whether a pane is folded stays egui_dock's own `collapsed` flag, set by its
//! own arrow: nothing here duplicates that bookkeeping, it only reads it.

use egui_dock::{DockState, Node, NodeIndex, Surface, SurfaceIndex, Tree};

use crate::panes::Tab;

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
    /// The window this fold was taken at, which is the window unfolding it owes
    /// back. [`Dial`] is runtime-only, so without this a project reopened with
    /// a pane already folded has no record of how wide the window has been —
    /// and the unfold's growth cap reads exactly that record, so an empty one
    /// holds the window at the width the fold left it.
    ///
    /// Zero in a blob written before it was recorded. There is no history to
    /// recover there, so the cap falls back to the window on screen, which is
    /// what it did for every blob then.
    #[serde(default)]
    window: f32,
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
    /// The fraction the fold last wrote into this split, so a difference at the
    /// top of the next frame is the USER's — a separator dragged, which
    /// [`Folds::absorb`] reads back into `fraction` before anything is derived
    /// from it. Without it every such drag is overwritten by the fold that owns
    /// the split, one frame after the handle highlighted for it.
    ///
    /// `None` until this fold has written anything, which is the first frame of
    /// it: the fraction in the tree is then still the user's own.
    ///
    /// Runtime-only, for the same reason as `rail`: nothing has been written
    /// into a tree that has only just been loaded.
    #[serde(skip)]
    written: Option<f32>,
}

/// Which child of a split is the folded one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Left,
    Right,
}

impl Side {
    /// The child of `node` on this side.
    fn of(self, node: NodeIndex) -> NodeIndex {
        match self {
            Side::Left => node.left(),
            Side::Right => node.right(),
        }
    }
}

/// One surface's layout as it stands, for the two readings that start from what
/// is on SCREEN rather than from what the layout is dialled at: the tree, which
/// surface of the dock it is, what the fold is holding there, and the window it
/// is being drawn in.
///
/// Read-only by construction, so the tree cannot be written to half way through
/// a derivation that is still reading it.
#[derive(Clone, Copy)]
struct Reading<'a> {
    tree: &'a Tree<Tab>,
    surface: SurfaceIndex,
    holds: &'a [Hold],
    style: &'a egui_dock::Style,
    area: f32,
}

/// A rail the user has pulled out into a pane again, and how wide they pulled
/// it (see [`grab`]).
///
/// Carried from the pull to the next frame rather than acted on where it
/// happens, because a width is not a layout: what the fold holds is the dialled
/// fraction the width is a share OF, and the dialled widths are known in
/// [`Folds::apply`] and nowhere else.
#[derive(Clone, Copy)]
struct Pull {
    surface: usize,
    /// The split holding the fold, which is what the width is a share of: a
    /// whole subtree can have folded into the one rail being pulled.
    node: usize,
    side: Side,
    /// The pane to bring back — the one whose stretch of the rail the pull
    /// started on, exactly as its own arrow would have.
    ///
    /// One pane rather than the subtree: a rail can hold panes that were folded
    /// separately, and down a column one of them may have been a tab bar long
    /// before the column folded sideways. Opening all of them would hand back
    /// panes nobody closed.
    leaf: usize,
    /// What the pane is to come back at, in points.
    width: f32,
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
        // A rail the user pulled open on the frame before this one (see
        // [`grab`]), which is where the width it was pulled to becomes a
        // fraction: the dialled layout lives here and nowhere else.
        let pull = dial.pull.take();
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
            // A floating surface has no window to ask, so its layout is simply
            // the one that fits the window it is in, every frame.
            let mut own = Dial::default();
            let dial = if main { &mut *dial } else { &mut own };
            // Two things arrive as a RENDERED layout — the one on screen, rails
            // and all — and have to be read back into the dialled one before
            // anything is derived from it: a separator the user dragged, and a
            // rail the user pulled open. Both only READ the tree, so the pull's
            // own opening waits until the reading is done with.
            let dragged = {
                let reading = Reading { tree, surface, holds: &holds, style, area };
                self.absorb(&reading)
            };
            // Still the fold the pull was aimed at, and the pane still folded
            // into it: the arrow, or a re-dock, can have had either back in the
            // frame between letting the pull go and this one.
            let mine = pull.filter(|pull| {
                pull.surface == surface.0
                    && holds.get(pull.node).map(|hold| hold.side) == Some(Some(pull.side))
                    && collapsed(tree, NodeIndex(pull.leaf))
            });
            // The pane opens first, so the width it was pulled to is priced
            // against the layout that will hold it — every fold still standing
            // inside the rail included.
            if let Some(pull) = &mine {
                uncollapse(tree, NodeIndex(pull.leaf));
            }
            let pulled = mine.is_some();
            // A pull clears collapsed flags, so which splits the fold is
            // holding is no longer what it was read as above.
            let holds = if pulled { self::holds(tree) } else { holds };
            let dialled = mine.and_then(|pull| {
                let split = NodeIndex(pull.node);
                let reading = Reading { tree, surface, holds: &holds, style, area };
                let child = pull.side.of(split);
                self.pulled(&reading, split, child, pull.width, dial.width).map(|at| (split, at))
            });
            if let Some((split, fraction)) = dialled {
                // Into the tree as well where there is no entry to hold it,
                // which is a fold that outlived the dial that measured it: the
                // entry is what unfolding reads, and without one the pane would
                // come back at the rail fraction still in the split.
                if !self.set_dialled(surface, split, fraction) {
                    set_fraction(tree, split, fraction);
                }
            }
            let moved = self.reconcile(tree, surface, &holds, area);
            // After the fold has handed back whatever it was holding, or the
            // fractions it wrote would still be in the tree on the frame the
            // flags say they are nobody's — which reads exactly like a drag.
            hold_floors(tree, &holds, style, &mut dial.seen, dial.gesturing);
            // The widest this window has been, for a session that was not there
            // to watch it get that wide: the folds came off the persist blob,
            // and each one remembers the window it was taken at.
            let remembered = self
                .0
                .iter()
                .filter(|fold| fold.surface == surface.0)
                .fold(0.0_f32, |widest, fold| widest.max(fold.window));
            if dial.widest <= 0.0 {
                dial.widest = remembered;
            }
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
            // A separator dragged re-dials for the same reason a window dragged
            // does: the layout is a different one now, and the window it would
            // need with every pane open moved with it. Left alone, the next fold
            // would price itself against the layout the drag replaced.
            let follow = (!settled && !moved) || dragged;
            let was = dial.width;
            if (follow && area > floor + 1.0) || dial.width <= 0.0 || refused {
                if let Some(dialled) = fit.dialled_for(area) {
                    dial.width = dialled;
                }
            }
            // A drag dials the layout wider — the panes on screen took width
            // between them, and the folded pane's share of the same split grew
            // with them, exactly as it would have if it were open — so the
            // ceiling an unfold may ask the window for rises by what the drag
            // added. Only by THAT: the ceiling is there to refuse the width a
            // fold banks when the window is dragged back out while it is folded,
            // which is width nobody asked for, and a drag is the user asking.
            // Left where it was, an unfold cannot pay for what the drag dialled
            // and the panes it dragged walk back on the way out.
            //
            // Held to the layout it is now dialled at, or a separator wiggled
            // back and forth would ratchet the ceiling up a drag at a time: each
            // pull outward adds to it and each pull back adds nothing, and the
            // guard is gone by the twentieth wiggle.
            if dragged && was > 0.0 {
                let asked = dial.widest + (dial.width - was).max(0.0);
                dial.widest = asked.min(dial.width.max(area));
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
                let widths = fitted.as_ref().unwrap_or(&widths);
                self.write_fractions(tree, surface, &holds, widths, separator);
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

    /// Carry a separator the user dragged back into the layout it is a
    /// rendering of. Answers whether one was.
    ///
    /// Every split with a fold below it has its fraction rewritten on the way
    /// out to the window (see [`Folds::write_fractions`]), so egui_dock's own
    /// drag lands on a number the next frame overwrites before the layout is
    /// derived from it: the handle highlights, the cursor changes, and no pane
    /// moves. What the drag names is a boundary in the RENDERED layout — the one
    /// on screen, rails and all — and what this holds is the dialled fraction
    /// that layout comes from, so the drag has to be read back through the
    /// derivation to survive.
    ///
    /// Read back by RUNNING the derivation rather than by inverting it: the
    /// rendered fraction rises with the dialled one, so a bisection over [`Fit`]
    /// finds the dialled fraction that renders where the user let go. The closed
    /// form exists, but one per shape of tree — a fold beside the dragged split
    /// solves differently from one inside it, and differently again from two —
    /// and every one of them is a second answer that has to agree with the
    /// derivation forever. Thirty halvings of the unit interval is a thousandth
    /// of a point in any window this runs in.
    fn absorb(&mut self, reading: &Reading) -> bool {
        let Reading { tree, surface, holds, style, .. } = *reading;
        let mut dragged = false;
        for (index, hold) in holds.iter().enumerate() {
            let node = NodeIndex(index);
            // Only splits whose fraction the fold has taken on its way out to
            // the window. A split that has folded a child of its OWN is
            // rendering a rail there, and a rail is a fixed number of points
            // that no fraction of this split can change — that separator is
            // [`grab`]'s, where dragging it means the pane coming back.
            if !hold.above || hold.folded() {
                continue;
            }
            let Some(fold) = self.0.iter().find(|fold| fold.is(surface, node)) else {
                continue;
            };
            let (Some(written), Node::Horizontal(split)) = (fold.written, &tree[node]) else {
                continue;
            };
            let target = split.fraction;
            // Neither the fraction the fold wrote nor what egui_dock's own clamp
            // makes of it: anything else in the split is the user's drag.
            let clamped = unmoved(written, split.rect.width(), style.separator.extra);
            if (target - written).abs() < 1e-5 || (target - clamped).abs() < 1e-5 {
                continue;
            }
            // Held to what the panes on either side need, which egui_dock's own
            // limit does not know: it holds the two children of this split apart
            // and nothing deeper in either of them (see [`min_widths`]).
            let target = floored(tree, node, target, style);
            if let Some(dialled) = self.solve(reading, node, target) {
                if let Some(fold) = self.0.iter_mut().find(|fold| fold.is(surface, node)) {
                    fold.fraction = dialled;
                    dragged = true;
                }
            }
        }
        dragged
    }

    /// The dialled fraction at `node` that renders `target`, or `None` where no
    /// fraction does: the bracket is that test, and it is the one that catches a
    /// separator with nothing on either side of it left to resize.
    ///
    /// Leaves the fold holding what it held — the answer is the caller's to
    /// write, and a search that walked away from its own scratch values would be
    /// a layout nobody dialled.
    fn solve(&mut self, reading: &Reading, node: NodeIndex, target: f32) -> Option<f32> {
        // Away from 0 and 1, where a split hands one child everything and the
        // derivation has nothing left to divide.
        let (mut low, mut high) = (0.002_f32, 0.998_f32);
        let was = self.dialled(reading.tree, reading.surface, node);
        let solved = (|| {
            let at_low = self.rendered_at(reading, node, low)?;
            let at_high = self.rendered_at(reading, node, high)?;
            // Outside the bracket no dialled fraction renders where the drag
            // asked, and bisecting anyway would converge on an endpoint and
            // report it as the answer.
            if at_low >= target || at_high <= target {
                return None;
            }
            for _ in 0..30 {
                let mid = 0.5 * (low + high);
                if self.rendered_at(reading, node, mid)? < target {
                    low = mid;
                } else {
                    high = mid;
                }
            }
            Some(0.5 * (low + high))
        })();
        self.set_dialled(reading.surface, node, was);
        solved
    }

    /// The fraction `node` comes out at ON SCREEN when it is dialled at
    /// `dialled`: the layout run forwards, which is what [`Folds::solve`]
    /// searches over. Matches [`Folds::write_fractions`], because it is
    /// answering the question that writes.
    fn rendered_at(&mut self, reading: &Reading, node: NodeIndex, dialled: f32) -> Option<f32> {
        let Reading { tree, surface, holds, style, area } = *reading;
        let separator = style.separator.width;
        self.set_dialled(surface, node, dialled);
        let fit = Fit::of(tree, holds, self, surface, style.tab_bar.height, separator);
        let widths = fit.dialled_for(area).and_then(|window| fit.widths(window))?;
        let (whole, before) = (widths.real[node.0], widths.real[node.left().0]);
        (whole > 0.0 && before.is_finite()).then(|| (before + separator * 0.5) / whole)
    }

    /// The dialled fraction at `split` that brings `child` back `width` points
    /// wide, once the window has caught up (see [`grab`]).
    ///
    /// Bisected over the derivation, exactly as a drag is ([`Folds::solve`]) and
    /// for the same reason: what a pull names is a width ON SCREEN, and between
    /// that and a fraction sits every fold still standing inside the rail — panes
    /// collapsed separately from this one, which stay collapsed and go on taking
    /// a rail each out of the width the pull asked for.
    ///
    /// Measured at the window the layout is DIALLED at rather than the one on
    /// screen: a pull is what asks for that window back, so the width it names is
    /// the width once it arrives. The window pays the difference between the
    /// pane and the rail, as it does for the arrow, and the panes beside it
    /// inside the split pay the rest.
    fn pulled(
        &mut self,
        reading: &Reading,
        split: NodeIndex,
        child: NodeIndex,
        width: f32,
        window: f32,
    ) -> Option<f32> {
        let (mut low, mut high) = (0.002_f32, 0.998_f32);
        let was = self.dialled(reading.tree, reading.surface, split);
        let solved = (|| {
            let (at_low, at_high) = (
                self.width_at(reading, split, child, low, window)?,
                self.width_at(reading, split, child, high, window)?,
            );
            // Which way the child grows: the fraction is the LEFT child's share,
            // so a right-hand child shrinks as it rises. Read off the ends rather
            // than off the side, which keeps this the same arithmetic either way.
            let rising = at_high > at_low;
            let (near, far) = (at_low.min(at_high), at_low.max(at_high));
            if width <= near || width >= far {
                // Wider (or narrower) than any fraction of this split can make
                // it: take the end of the range rather than nothing, so a pull
                // that overshoots opens the pane as far as it can go.
                return Some(if (width >= far) == rising { high } else { low });
            }
            for _ in 0..30 {
                let mid = 0.5 * (low + high);
                let at = self.width_at(reading, split, child, mid, window)?;
                if (at < width) == rising {
                    low = mid;
                } else {
                    high = mid;
                }
            }
            Some(0.5 * (low + high))
        })();
        self.set_dialled(reading.surface, split, was);
        solved
    }

    /// The width `child` comes out at ON SCREEN when its split is dialled at
    /// `fraction` — the same derivation [`Folds::rendered_at`] reads a fraction
    /// out of, asked for a width instead.
    fn width_at(
        &mut self,
        reading: &Reading,
        split: NodeIndex,
        child: NodeIndex,
        fraction: f32,
        window: f32,
    ) -> Option<f32> {
        let Reading { tree, surface, holds, style, area } = *reading;
        self.set_dialled(surface, split, fraction);
        let fit = Fit::of(tree, holds, self, surface, style.tab_bar.height, style.separator.width);
        // A floating surface carries no dial, so the window its layout is
        // dialled at is the one that fits the window it is in.
        let window = match window > 0.0 {
            true => window,
            false => fit.dialled_for(area)?,
        };
        Some(fit.widths(window)?.real[child.0])
    }

    /// Hold a split at a dialled fraction, answering whether there was an entry
    /// to hold it — a split the fold is not holding has no fraction to keep.
    fn set_dialled(&mut self, surface: SurfaceIndex, node: NodeIndex, fraction: f32) -> bool {
        match self.0.iter_mut().find(|fold| fold.is(surface, node)) {
            Some(fold) => {
                fold.fraction = fraction;
                true
            }
            None => false,
        }
    }

    /// Write the fraction every split needs for the widths [`Fit`] worked out,
    /// and keep each one: a difference at the top of the next frame is then a
    /// separator the user dragged (see [`Folds::absorb`]).
    ///
    /// Only splits with a fold under them are touched: everywhere else the
    /// fraction in the tree IS the dialled one, and rewriting it from a width
    /// would only round it off its mark.
    fn write_fractions(
        &mut self,
        tree: &mut Tree<Tab>,
        surface: SurfaceIndex,
        holds: &[Hold],
        widths: &Widths,
        separator: f32,
    ) {
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
            let fraction = ((before + separator * 0.5) / whole).clamp(0.0, 1.0);
            set_fraction(tree, node, fraction);
            if let Some(fold) = self.0.iter_mut().find(|fold| fold.is(surface, node)) {
                fold.written = Some(fraction);
            }
        }
    }

    /// Take the fraction of every split that has just folded, and give back the
    /// fraction of every split that has just unfolded. Answers whether either
    /// happened, which is what decides if the window has to move.
    fn reconcile(
        &mut self,
        tree: &mut Tree<Tab>,
        surface: SurfaceIndex,
        holds: &[Hold],
        area: f32,
    ) -> bool {
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
            // `area` is still the pre-fold window here — the resize has been
            // decided and not yet asked for — which is the width this fold is
            // about to take off it, and the width unfolding it owes back.
            self.0.push(Fold {
                surface: surface.0,
                node: index,
                fraction: split.fraction,
                rail,
                window: area,
                written: None,
            });
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
    /// A rail pulled open, waiting for the frame that can price it. Set by
    /// [`paint`], which is where the pull is let go of, and taken by
    /// [`Folds::apply`], which is where a width becomes a fraction.
    pull: Option<Pull>,
    /// Every split's fraction as this pass last left it, so the next one can tell
    /// a separator the user DRAGGED from one egui_dock has re-clamped behind its
    /// back — see [`hold_floors`].
    seen: Vec<f32>,
    /// Whether a pointer was down (or has just come up) when this frame's input
    /// was read, which is what tells those two apart: only a gesture moves a
    /// fraction on purpose.
    gesturing: bool,
}

impl Dial {
    /// What the shell saw of the pointer this frame, before [`Folds::apply`] reads
    /// the fractions the last one left behind.
    pub fn watch_pointer(&mut self, gesturing: bool) {
        self.gesturing = gesturing;
    }
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

/// The narrowest each node can be DRAWN: a pane's own floor, a rail for whatever
/// is folded, and for a split whatever its two children need side by side.
///
/// egui_dock's own limit (`separator.extra`) is a single number applied to the two
/// children of whichever split is being dragged — so a pane deeper inside either
/// of them keeps none of it: drag the boundary between a picture pair and the
/// settings column far enough and the pair stops at one pane's floor, with the two
/// panes inside it sharing that between them, 71 points and 27. What a floor means
/// is that no PANE is drawn below it, wherever it sits in the tree, which is this
/// sum.
///
/// Indexed by node. Bottom-up, which a reverse walk is: a node's children are
/// always further along than it is.
fn min_widths(tree: &Tree<Tab>, floor: f32, rail: f32, separator: f32) -> Vec<f32> {
    let mut mins = vec![0.0; tree.len()];
    for index in (0..tree.len()).rev() {
        let node = NodeIndex(index);
        // Folded, and drawn as the rails it holds: folding is how a pane gets
        // smaller than the floor, and a rail is what it gets instead.
        if tree[node].is_collapsed() {
            mins[index] = rail_span(rail_columns(tree, node), rail, separator);
            continue;
        }
        let (left, right) = (node.left(), node.right());
        mins[index] = match &tree[node] {
            Node::Leaf(_) => floor,
            Node::Horizontal(_) if right.0 < tree.len() => {
                mins[left.0] + separator + mins[right.0]
            }
            Node::Vertical(_) if right.0 < tree.len() => mins[left.0].max(mins[right.0]),
            _ => 0.0,
        };
    }
    mins
}

/// Hold every pane to its floor against a separator the user has just dragged,
/// for the splits the fold does not own (its own are held in [`Folds::absorb`],
/// where a drag on them is read).
///
/// Only fractions that have MOVED, which is only ever a drag: a window resize
/// leaves every fraction where it was and re-derives the widths from it, so a
/// floor enforced against every frame would take a narrow window as licence to
/// re-dial the layout — and a layout that quietly rearranges itself when the
/// window moves is the thing [`Folds`] is built to prevent.
fn hold_floors(
    tree: &mut Tree<Tab>,
    holds: &[Hold],
    style: &egui_dock::Style,
    seen: &mut Vec<f32>,
    gesturing: bool,
) {
    let separator = style.separator.width;
    let floor = style.separator.extra - separator * 0.5;
    let fraction_of = |tree: &Tree<Tab>, node: NodeIndex| match &tree[node] {
        Node::Horizontal(split) | Node::Vertical(split) => Some(split.fraction),
        _ => None,
    };
    // A tree that has changed shape has no history to compare against, and
    // re-docking is not a drag: take this pass to write one down.
    if seen.len() != tree.len() {
        *seen = (0..tree.len())
            .map(|index| fraction_of(tree, NodeIndex(index)).unwrap_or(f32::NAN))
            .collect();
        return;
    }
    let mins = min_widths(tree, floor, style.tab_bar.height, separator);
    let was = seen.clone();
    for (index, was) in was.iter().enumerate() {
        let node = NodeIndex(index);
        let (left, right) = (node.left(), node.right());
        let Some(fraction) = fraction_of(tree, node) else {
            continue;
        };
        // Widths, so only the splits that divide any. A pane's height keeps
        // egui_dock's own limit, which bounds the two panes either side of the
        // separator being dragged and no others.
        // Not the fold's own: it rewrites those every frame to hold a rail at a
        // rail's width, which is below every floor here by construction, and a
        // drag on one of them is read (and held) in [`Folds::absorb`] instead.
        let owned = holds.get(index).is_some_and(Hold::held);
        let moved = (fraction - was).abs() > 1e-6;
        let along = tree[node].rect().map(|rect| match tree[node].is_horizontal() {
            true => rect.width(),
            false => rect.height(),
        });
        // Moved with nobody touching it: egui_dock re-clamps every separator's
        // fraction on every frame, dragged or not, to keep `separator.extra`
        // points of pane on either side — so a window narrow enough for that to
        // bite walks the layout toward 50/50 by itself, a pane at a time, and
        // what the user dialled is gone for good. The layout it clamped was
        // already drawn (the clamp lands after `compute_rect_sizes`), so putting
        // the fraction back before the next one costs nothing at all.
        if !owned && moved && !gesturing {
            let range = along.unwrap_or(0.0);
            if (fraction - unmoved(*was, range, style.separator.extra)).abs() < 1e-6 {
                set_fraction(tree, node, *was);
                continue;
            }
        }
        let dragged = !owned && moved && gesturing;
        if dragged && tree[node].is_horizontal() && right.0 < tree.len() {
            if let Some(size) = along {
                let (before, after) = (mins[left.0], mins[right.0]);
                // Room for both, or there is nothing to hold either of them to:
                // a window too narrow for the panes it holds is not a drag's
                // doing, and refusing to move at all would be worse.
                if before + separator + after <= size && size > 0.0 {
                    let low = (before + separator * 0.5) / size;
                    let high = 1.0 - (after + separator * 0.5) / size;
                    set_fraction(tree, node, fraction.clamp(low.min(high), high.max(low)));
                }
            }
        }
        // Left where it was while the fold owns the split, so that the fraction
        // the fold hands BACK on the way out reads as the one the user dialled
        // rather than as a drag: what is in the tree meanwhile is a rail's, and
        // comparing against that would take every unfold for a gesture.
        if !owned {
            seen[index] = fraction_of(tree, node).unwrap_or(f32::NAN);
        }
    }
}

/// A dragged fraction held to what the panes on either side of the split need —
/// the same sum [`hold_floors`] holds the fold's own splits to, for the ones whose
/// drag arrives as a rendered fraction instead ([`Folds::absorb`]).
fn floored(
    tree: &Tree<Tab>,
    node: NodeIndex,
    fraction: f32,
    style: &egui_dock::Style,
) -> f32 {
    let separator = style.separator.width;
    let floor = style.separator.extra - separator * 0.5;
    let (left, right) = (node.left(), node.right());
    let Some(rect) = tree[node].rect() else {
        return fraction;
    };
    if right.0 >= tree.len() || rect.width() <= 0.0 {
        return fraction;
    }
    let mins = min_widths(tree, floor, style.tab_bar.height, separator);
    let (size, before, after) = (rect.width(), mins[left.0], mins[right.0]);
    if before + separator + after > size {
        return fraction;
    }
    let low = (before + separator * 0.5) / size;
    let high = 1.0 - (after + separator * 0.5) / size;
    fraction.clamp(low.min(high), high.max(low))
}

/// The fraction a split comes back off the tree with when nobody dragged its
/// separator, which is not always the one the fold wrote there.
///
/// egui_dock re-clamps every separator's fraction on every frame, dragged or
/// not, to keep `separator.extra` points of pane on either side of it. A rail is
/// narrower than that by construction, so a fold's own fractions come back
/// clamped — harmless to the layout, which is computed before the clamp and
/// rewritten after it, and indistinguishable from a drag unless it is named. A
/// phantom one would re-dial the whole layout every frame a pane spent folded.
fn unmoved(written: f32, range: f32, extra: f32) -> f32 {
    if range <= 0.0 {
        return written;
    }
    let min = (extra / range).min(1.0);
    let max = 1.0 - min;
    written.clamp(min.min(max), max.max(min))
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

mod paint;
pub use paint::paint;

#[cfg(test)]
mod tests;
