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
    /// The separator this gesture has hold of, noted at the press and read at the
    /// release — see [`collapse_at_floor`].
    grabbed: Option<Grab>,
}

/// A separator a press landed on: which split it divides, and whether the pane
/// beside it was already at its floor when the press happened.
///
/// Noted at the press because that is the last moment the separator is under the
/// pointer — a drag moves it away from where it started — and because "was it
/// already there" is the difference between a pane the user has just squeezed to
/// nothing and one the window squeezed for them.
#[derive(Clone, Copy)]
struct Grab {
    surface: usize,
    node: usize,
    floored: bool,
    /// The fraction the split was dialled at before the gesture, which is what a
    /// pane folded DOWNWARDS comes back at.
    ///
    /// Sideways the fold banks the width the drag left, and the rail's own handle
    /// is how a pane comes back at a width the user picks ([`grab`]). Downwards
    /// there is no such handle — egui_dock draws no separator beside a pane folded
    /// down — and no window paying for it either, so the arrow is the only way
    /// back and a fraction left at the floor would hand back a pane exactly as
    /// tall as the tab bar it was folded to: open, and indistinguishable from
    /// folded. The height it had before the drag is the only useful answer, and
    /// costs nothing to keep, the fraction of a collapsed vertical split being
    /// invisible until it opens.
    fraction: f32,
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
pub fn paint(
    ui: &egui::Ui,
    dock: &mut DockState<Tab>,
    style: &egui_dock::Style,
    dial: &mut Dial,
) {
    let rail = style.tab_bar.height;
    // Frameless mode hides every tab bar, which takes the rail with it: a fold
    // there is a pane squeezed to NOTHING, so there is no rail to draw, no name
    // to put up it and no arrow to bring the pane back with. What is still there
    // is the separator the pane left behind, and the panes on either side of it
    // that a drag on it means (see the handles below) — so the chrome is what
    // gets skipped, not the frame.
    let chrome = rail > 0.0;
    // The pane an arrow of ours was clicked for, and the split a pinned
    // separator passed a drag out to: both applied once the tree is no longer
    // being read from.
    let mut opened = None;
    let mut shoved = None;
    for index in 0..dock.surfaces_count() {
        let surface = SurfaceIndex(index);
        let Some(tree) = dock.get_surface(surface).and_then(Surface::node_tree) else {
            continue;
        };
        let holds = holds(tree);
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
            let folded = side.of(node);
            let Some(rect) = tree[folded].rect() else {
                continue;
            };
            // Nothing here until the fold has actually been laid out: on the
            // frame a pane is collapsed on it is still its old width, and a
            // rail's worth of chrome across a whole pane would flash, while a
            // handle would sit where the pane's edge is about to stop being.
            //
            // A rail's width is the measure of settled, and in frameless mode
            // that is zero — so the slack there is a couple of points of
            // rounding rather than a rail's worth of it, and the rect is allowed
            // to have no width at all.
            let span = rail_span(rail_columns(tree, folded), rail, style.separator.width);
            if rect.height() <= 0.0 || rect.width() < 0.0 {
                continue;
            }
            if rect.width() >= span + rail.max(2.0) {
                continue;
            }
            if chrome {
                for band in name_bands(tree, folded) {
                    if paint_band(ui, &band, side, surface, style) {
                        opened = Some((surface, band.node));
                    }
                }
            }
            // Every separator this fold has pinned: the one the rail sits
            // against, and any between the rails of a folded pair. None of them
            // can move what is immediately on either side of it — a rail is a
            // fixed number of points — so they all resize the same thing, the
            // nearest open pane on each side, by passing the drag outward to the
            // split that divides those two (see [`shove_target`]).
            let outer = match side {
                Side::Left => rect.right()..=rect.right() + style.separator.width,
                Side::Right => rect.left() - style.separator.width..=rect.left(),
            };
            let outer = egui::Rect::from_x_y_ranges(outer, rect.y_range());
            let target = shove_target(tree, &holds, node);
            for (index, band) in
                std::iter::once(outer).chain(inner_bands(tree, folded)).enumerate()
            {
                let id = egui::Id::new(("fold band", surface.0, node.0, index));
                match (target, index) {
                    // Somewhere outward to pass it: the panes move as the drag
                    // goes, which is what any separator between two panes does.
                    (Some(target), _) => {
                        if let Some(delta) = shove(ui, band, id, style) {
                            shoved = Some((surface, target, delta));
                        }
                    }
                    // Nowhere: the fold is holding the whole of one side of the
                    // window, so the only width that can move is the folded
                    // pane's own, out of the window (see [`grab`]). The
                    // separator the rail sits against is the one that offers
                    // that; a band between two rails offers nothing.
                    //
                    // A pane at a time, in the stretch of the separator beside
                    // that pane's stretch of the rail, so a pull opens what its
                    // own arrow would (see [`name_bands`]).
                    (None, 0) => {
                        for pane in name_bands(tree, folded) {
                            let slice = egui::Rect::from_x_y_ranges(
                                band.x_range(),
                                pane.rect.y_range(),
                            );
                            let id = id.with(pane.node.0);
                            let split = tree[node].rect();
                            if let Some(width) = grab(ui, slice, side, split, span, id, style) {
                                dial.pull = Some(Pull {
                                    surface: surface.0,
                                    node: node.0,
                                    side,
                                    leaf: pane.node.0,
                                    width,
                                });
                            }
                        }
                    }
                    (None, _) => deaden(ui, band, style),
                }
            }
        }
    }
    if let Some((surface, node)) = opened {
        if let Some(tree) = dock.get_surface_mut(surface).and_then(Surface::node_tree_mut) {
            uncollapse(tree, node);
        }
    }
    if let Some((surface, node, delta)) = shoved {
        if let Some(tree) = dock.get_surface_mut(surface).and_then(Surface::node_tree_mut) {
            nudge(tree, node, delta, style);
        }
    }
}

/// The split a separator a fold has pinned passes its drag out to: the nearest
/// one above the fold that still divides two panes both of which can change
/// width, which is the boundary the user is pointing at whether they know the
/// tree or not.
///
/// A separator with a rail on either side of it cannot move what it divides, and
/// neither can the split that folded the rail — its fraction is rewritten every
/// frame to hold the rail at a rail's width. What the user sees is a boundary
/// with a pane somewhere to the left of it and a pane somewhere to the right,
/// with pinned chrome in between, and dragging it moves those two: the rails
/// travel with the boundary, keeping their width, and the open panes trade.
///
/// `None` where the fold is holding the whole of one side of the window, so the
/// only pane that can change width is the folded one itself.
fn shove_target(tree: &Tree<Tab>, holds: &[Hold], fold: NodeIndex) -> Option<NodeIndex> {
    let mut node = fold;
    while let Some(parent) = node.parent() {
        let held = holds.get(parent.0).copied().unwrap_or_default();
        // Horizontal, because a vertical split divides height and has no share
        // of the width to trade; and not itself folded, or its fraction is the
        // fold's to write and a drag would be overwritten again.
        if held.above && !held.folded() && tree[parent].is_horizontal() {
            return Some(parent);
        }
        node = parent;
    }
    None
}

/// Move a split's boundary by `delta` points, as egui_dock's own separator drag
/// does it: the fraction moves by the drag over the split's width on screen, and
/// is clamped to keep `separator.extra` points of pane on either side.
///
/// Written straight into the tree, where the next frame reads it as the drag it
/// is ([`Folds::absorb`]) — which is also how egui_dock's separator hands a drag
/// over, so a shoved boundary and a dragged one arrive by the same door.
fn nudge(tree: &mut Tree<Tab>, node: NodeIndex, delta: f32, style: &egui_dock::Style) {
    let Some(rect) = tree[node].rect() else {
        return;
    };
    let range = rect.width();
    if range <= 0.0 {
        return;
    }
    let min = (style.separator.extra / range).min(1.0);
    let max = 1.0 - min;
    let (min, max) = (min.min(max), max.max(min));
    if let Node::Horizontal(split) = &mut tree[node] {
        split.fraction = (split.fraction + delta / range).clamp(min, max);
    }
}

/// A separator a fold has pinned, as the resize handle for the boundary it
/// stands on: it paints and reads like any other separator, and the drag goes to
/// the split that can actually move (see [`shove_target`]).
///
/// Answers the points the boundary has moved this frame, live, because there is
/// nothing to defer: the panes it trades are both on screen.
fn shove(
    ui: &egui::Ui,
    band: egui::Rect,
    id: egui::Id,
    style: &egui_dock::Style,
) -> Option<f32> {
    let reach = egui::vec2(style.separator.extra_interact_width * 0.5, 0.0);
    let response = ui
        .interact(band.expand2(reach), id, egui::Sense::click_and_drag())
        .on_hover_and_drag_cursor(egui::CursorIcon::ResizeHorizontal);
    let color = if response.dragged() {
        style.separator.color_dragged
    } else if response.hovered() {
        style.separator.color_hovered
    } else {
        style.separator.color_idle
    };
    ui.painter().rect_filled(band, egui::CornerRadius::ZERO, color);
    let delta = response.drag_delta().x;
    (response.dragged() && delta != 0.0).then_some(delta)
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

/// The separator beside a rail, as the resize handle of the pane the rail stands
/// in for. Answers the width that pane is to come back at, once, on the frame the
/// pull is let go of.
///
/// For the fold that has nowhere to pass a drag outward to ([`shove_target`]),
/// which is one holding the whole of one side of the window: the pane beside the
/// rail cannot grow, because there is nothing on the other side of it to take the
/// width from but the window itself. What CAN move is the folded pane's own
/// width, out of the window, which is the fold run backwards at a width the user
/// chose rather than the one they folded at — so that is what this offers. See
/// [`Folds::pulled`] for who pays.
///
/// The pane opens when the drag ENDS. Opening it as the pull went would hand the
/// rest of the gesture to a separator that has just replaced this handle —
/// egui_dock's own, for a split that is no longer folded — and drop the pull
/// half way through it.
fn grab(
    ui: &egui::Ui,
    band: egui::Rect,
    side: Side,
    split: Option<egui::Rect>,
    span: f32,
    id: egui::Id,
    style: &egui_dock::Style,
) -> Option<f32> {
    let split = split?;
    let reach = egui::vec2(style.separator.extra_interact_width * 0.5, 0.0);
    let response = ui
        .interact(band.expand2(reach), id, egui::Sense::click_and_drag())
        .on_hover_and_drag_cursor(egui::CursorIcon::ResizeHorizontal);
    // Hover and drag accents as egui_dock paints them on a live separator,
    // because that is what this is now — the fold took the accent away with the
    // drag, and a handle that does something has to look like one.
    let color = if response.dragged() {
        style.separator.color_dragged
    } else if response.hovered() {
        style.separator.color_hovered
    } else {
        style.separator.color_idle
    };
    ui.painter().rect_filled(band, egui::CornerRadius::ZERO, color);
    // What the pane would come back at from where the pointer is: never
    // narrower than the rail standing in for it, since below that it is still
    // folded, and never wider than the split the rail sits in, less a rail for
    // the pane beside it — a pointer that has run out of split has run out of
    // gesture, and the pane can be dragged wider once it is a pane again.
    let width = |at: egui::Pos2| {
        let pulled = match side {
            Side::Left => at.x - split.left(),
            Side::Right => split.right() - at.x,
        };
        pulled.clamp(span, (split.width() - style.separator.width - span).max(span))
    };
    let at = response.interact_pointer_pos()?;
    if response.dragged() {
        // How much pane is being asked for, while the pull is still in hand: the
        // rail cannot follow the pointer, being a rail until the pane opens, so
        // without the line a pull has no answer at all until it is let go.
        //
        // The line is the pane's far edge measured from the rail's OUTER side,
        // which is the side the width is counted from. The window grows behind
        // it by what the fold took, so a pane pulled out of a rail on the left
        // lands its edge exactly here and one pulled out of a rail on the right
        // lands that much further out, having grown into the width the window
        // gave back rather than over the pane the pull crossed.
        let edge = match side {
            Side::Left => split.left() + width(at),
            Side::Right => split.right() - width(at),
        };
        let half = style.separator.width * 0.5;
        let guide = egui::Rect::from_x_y_ranges(edge - half..=edge + half, split.y_range());
        ui.painter().rect_filled(guide, egui::CornerRadius::ZERO, style.separator.color_dragged);
        return None;
    }
    if !response.drag_stopped() {
        return None;
    }
    // A pull that ends where it started is a click on a separator, and one aimed
    // INTO the rail is a pane that stays folded. Both come out at the clamp's
    // floor, which is the rail's own width.
    let width = width(at);
    (width > span + 1.0).then_some(width)
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

/// Fold a pane away, as egui_dock's own arrow would have — the mirror of
/// [`uncollapse`], and private for the same reason.
///
/// A split becomes collapsed when BOTH its children are, and each one's count of
/// collapsed leaves is remade from the two below it: a horizontal split holds as
/// many tab bars as its deeper side, a vertical one as many as both.
fn collapse(tree: &mut Tree<Tab>, leaf: NodeIndex) {
    tree[leaf].set_collapsed(true);
    let mut child = leaf;
    while let Some(parent) = child.parent() {
        let (left, right) = (parent.left(), parent.right());
        let (below, beside) =
            (tree[left].collapsed_leaf_count(), tree[right].collapsed_leaf_count());
        if tree[left].is_collapsed() && tree[right].is_collapsed() {
            tree[parent].set_collapsed(true);
        }
        let leaves = if tree[parent].is_horizontal() { below.max(beside) } else { below + beside };
        tree[parent].set_collapsed_leaf_count(leaves);
        child = parent;
    }
}

/// Fold a pane that has been dragged down to the size a fold would leave it at
/// anyway, once the drag is let go of.
///
/// The floor a separator can be dragged to IS a folded pane's own size (see
/// `theme::drag_floor`), so without this the dock has two states that look the
/// same and behave differently: a pane at its floor draws a tab bar's worth of
/// nothing, keeps its share of the window, and has none of a rail's name or
/// arrow, while the fold beside it has all three and hands its width back.
/// Folding what the user has already dragged to nothing turns the lookalike into
/// the real thing, and because the two sizes are the same number, nothing on
/// screen moves when it does — not the pane, not its neighbours, not the window.
///
/// On the RELEASE, not on reaching the floor: a fold mid-drag ends the gesture,
/// since egui identifies a separator by a per-frame allocation counter and a
/// collapsing leaf changes what the dock allocates before it, so the drag is
/// dropped exactly where the user might want to pull back out of it.
///
/// What makes the pane's floor the user's doing is the gesture that put it there,
/// so the separator is noted at the PRESS ([`Grab`]) — by then it is still under
/// the pointer, which it stops being the moment the drag moves it — along with
/// whether the pane was already at its floor before the gesture began. A pane can
/// reach its floor with no drag at all, a window dragged narrow enough squeezing
/// every pane it holds, and folding those would be the window quietly rearranging
/// the layout, which is the one thing [`Folds`] exists to avoid.
pub fn collapse_at_floor(
    ui: &egui::Ui,
    dock: &mut DockState<Tab>,
    style: &egui_dock::Style,
    dial: &mut Dial,
) {
    let (pressed, released, at) = ui.input(|i| {
        (i.pointer.any_pressed(), i.pointer.any_released(), i.pointer.press_origin())
    });
    if pressed {
        dial.grabbed = at.and_then(|at| grabbed(dock, style, at));
    }
    if !released {
        return;
    }
    // A gesture that found the pane already at its floor is not the one that put
    // it there: a press on that separator, however brief, must not fold it.
    let Some(grab) = dial.grabbed.take().filter(|grab| !grab.floored) else {
        return;
    };
    let surface = SurfaceIndex(grab.surface);
    let Some(tree) = dock.get_surface_mut(surface).and_then(Surface::node_tree_mut) else {
        return;
    };
    let node = NodeIndex(grab.node);
    let Some(leaf) = at_floor(tree, node, style) else {
        return;
    };
    collapse(tree, leaf);
    // Downwards, the height the pane had before the drag is what its arrow gives
    // back (see [`Grab::fraction`]).
    if let Node::Vertical(split) = &mut tree[node] {
        split.fraction = grab.fraction.clamp(0.0, 1.0);
    }
}

/// The separator a press has taken hold of, if it landed on one: which split it
/// divides, and whether the pane beside it was at its floor already.
fn grabbed(dock: &DockState<Tab>, style: &egui_dock::Style, at: egui::Pos2) -> Option<Grab> {
    let reach = style.separator.extra_interact_width * 0.5;
    for index in 0..dock.surfaces_count() {
        let surface = SurfaceIndex(index);
        let Some(tree) = dock.get_surface(surface).and_then(Surface::node_tree) else {
            continue;
        };
        for node in 0..tree.len() {
            let node = NodeIndex(node);
            let (left, right) = (node.left(), node.right());
            if right.0 >= tree.len() || !tree[node].is_parent() {
                continue;
            }
            // Downwards, a folded pane has no separator to drag back out of
            // (egui_dock draws none beside a collapsed leaf in a vertical split),
            // so there the arrow is what undoes this — which is fine with a tab
            // bar to click and a trap without one.
            if tree[node].is_vertical() && style.tab_bar.height <= 0.0 {
                continue;
            }
            let (Some(before), Some(after)) = (tree[left].rect(), tree[right].rect()) else {
                continue;
            };
            let band = match tree[node].is_horizontal() {
                true => {
                    egui::Rect::from_x_y_ranges(before.right()..=after.left(), before.y_range())
                }
                false => {
                    egui::Rect::from_x_y_ranges(before.x_range(), before.bottom()..=after.top())
                }
            };
            if !band.expand(reach).contains(at) {
                continue;
            }
            return Some(Grab {
                surface: surface.0,
                node: node.0,
                floored: at_floor(tree, node, style).is_some(),
                fraction: match &tree[node] {
                    Node::Horizontal(split) | Node::Vertical(split) => split.fraction,
                    _ => 0.5,
                },
            });
        }
    }
    None
}

/// The child of `node` that is an open pane squeezed to its floor, if exactly one
/// of the two is.
///
/// Both at once is a split with nothing left in it to divide, and folding either
/// would be a guess at which; a child that is a whole subtree is not folded here
/// either, since that would take every pane in it down with a drag aimed at one
/// boundary.
fn at_floor(tree: &Tree<Tab>, node: NodeIndex, style: &egui_dock::Style) -> Option<NodeIndex> {
    // egui_dock clamps a split's fraction to keep `extra` points on either side
    // of the separator, and a child's share of that is `extra` less the half
    // separator the split takes off it.
    let floor = style.separator.extra - style.separator.width * 0.5;
    let (left, right) = (node.left(), node.right());
    if right.0 >= tree.len() {
        return None;
    }
    let sitting = |child: NodeIndex| {
        let Some(rect) = tree[child].rect() else {
            return false;
        };
        let size = if tree[node].is_horizontal() { rect.width() } else { rect.height() };
        tree[child].is_leaf() && !tree[child].is_collapsed() && size <= floor + 0.5
    };
    match (sitting(left), sitting(right)) {
        (true, false) => Some(left),
        (false, true) => Some(right),
        _ => None,
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

/// Take the grab handle off a separator with a rail on both sides of it AND
/// nothing outward to pass a drag to — inside a fold that is holding the whole of
/// one side of the window, where there is no second open pane for the boundary to
/// trade against.
///
/// egui_dock keeps drawing them, hover accent and resize cursor and all, and
/// dragging one cannot do anything: neither side of it can change width, the fold
/// rewrites the fraction it would set on the very next frame, and here there is
/// no boundary further out that the drag would mean instead. So the invitation is
/// withdrawn — the same thing egui_dock does for a pane folded downwards, which
/// simply has no separator at all.
///
/// Every other separator a fold has pinned does something: [`shove`] where there
/// is a boundary outward to move, and [`grab`] on the rail's open side where there
/// is not.
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

    /// A blob written before a fold recorded its window still loads. There is
    /// no history in it to recover, so the entry comes back with none — which
    /// is what every blob had before the field existed.
    #[test]
    fn a_fold_blob_predating_the_recorded_window_still_loads() {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        collapse(&mut dock, Tab::Lattice, true);
        let _ = settle(&mut folds, &mut dock, &mut dial, 1000.0);

        let saved = ron::to_string(&folds).expect("folds serialize");
        assert!(saved.contains("window:"), "the field must be there to strip");
        // The same blob as a build that never wrote the field: drop the key and
        // the number after it, leaving every other field where it was.
        let stale: String = saved
            .split(",window:")
            .enumerate()
            .map(|(i, part)| match i {
                0 => part.to_string(),
                _ => part[part.find([',', ')']).unwrap_or(part.len())..].to_string(),
            })
            .collect();
        assert_ne!(stale, saved, "the strip must have removed something");

        let loaded: Folds = ron::from_str(&stale).expect("a blob without the field still loads");
        assert_eq!(loaded.0.len(), folds.0.len(), "every entry survives");
        assert!(loaded.0.iter().all(|fold| fold.window == 0.0), "no history to recover");
    }

    /// [`Folds`] is persisted and [`Dial`] is not, so a project reopened with a
    /// pane folded sideways starts with no record of how wide the window has
    /// been. The unfold's growth cap reads that record, and an empty one caps
    /// growth at nothing — so the pane opens back into a window still the width
    /// the fold left it, which is the one thing persisting the fold is for.
    #[test]
    fn a_fold_that_outlived_the_editor_window_still_unfolds_to_its_layout() {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        collapse(&mut dock, Tab::Lattice, true);
        let folded = settle(&mut folds, &mut dock, &mut dial, 1000.0);

        // Reopening: the dock and its folds come back off the persist blob,
        // the dial does not, and the host restores the window the fold left.
        let mut dial = Dial::default();
        let _ = frame(&mut folds, &mut dock, &mut dial, folded);
        collapse(&mut dock, Tab::Lattice, false);
        let reopened = settle(&mut folds, &mut dock, &mut dial, folded);
        assert!(
            (reopened - 1000.0).abs() < 1.0,
            "unfolding a persisted fold left the window at {reopened}, not the 1000 it came from"
        );
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

    /// A separator dragged `delta` points, as egui_dock's own drag does it: the
    /// fraction moves by the drag over the split's width ON SCREEN, and is then
    /// re-clamped to keep `separator.extra` points of pane on either side of it.
    ///
    /// The clamp is part of the drag rather than a detail of it: egui_dock
    /// applies it on every frame, dragged or not, and telling it apart from a
    /// drag is the whole of what `unmoved` is for.
    fn drag(dock: &mut DockState<Tab>, node: NodeIndex, delta: f32) {
        let extra = style().separator.extra;
        let range = width(dock, node);
        let min = (extra / range).min(1.0);
        let max = 1.0 - min;
        let (min, max) = (min.min(max), max.max(min));
        match &mut dock[SurfaceIndex::main()][node] {
            Node::Horizontal(split) | Node::Vertical(split) => {
                split.fraction = (split.fraction + delta / range).clamp(min, max);
            }
            _ => panic!("{node:?} is not a split"),
        }
    }

    /// egui_dock's per-frame re-clamp on every separator, with nobody dragging
    /// anything — which is what the tree looks like at the top of every frame a
    /// pane spends folded.
    fn reclamp(dock: &mut DockState<Tab>) {
        for index in 0..dock[SurfaceIndex::main()].len() {
            let node = NodeIndex(index);
            if dock[SurfaceIndex::main()][node].is_parent() {
                drag(dock, node, 0.0);
            }
        }
    }

    /// The one resize a folded pane's own separator can mean, where there is no
    /// boundary outward to pass a drag to: the pane coming back at the width it
    /// is pulled out to. `paint` reads the pull off the pointer; this is the half
    /// that prices it.
    fn pull(dial: &mut Dial, node: NodeIndex, side: Side, leaf: NodeIndex, width: f32) {
        dial.pull = Some(Pull { surface: 0, node: node.0, side, leaf: leaf.0, width });
    }

    /// Four panes in a row: `split_right` down the right, so every split is a
    /// horizontal one and the two in the middle can be folded independently.
    fn row() -> DockState<Tab> {
        let mut dock = DockState::new(vec![Tab::Lattice]);
        let surface = dock.main_surface_mut();
        let [_, rest] = surface.split_right(NodeIndex::root(), 0.4, vec![Tab::Spectral]);
        let [_, rest] = surface.split_right(rest, 0.34, vec![Tab::Tuning]);
        surface.split_right(rest, 0.5, vec![Tab::Notes]);
        dock
    }

    /// The same row, but with the two collapsed panes SIBLINGS: their split is
    /// collapsed too, so one fold renders as two rails side by side rather than
    /// two folds rendering one each.
    fn pair_row() -> DockState<Tab> {
        let mut dock = DockState::new(vec![Tab::Lattice]);
        let surface = dock.main_surface_mut();
        let [_, right] = surface.split_right(NodeIndex::root(), 0.4, vec![Tab::Spectral]);
        let [pair, _] = surface.split_right(right, 0.5, vec![Tab::Notes]);
        surface.split_right(pair, 0.5, vec![Tab::Tuning]);
        dock
    }

    #[test]
    fn a_folded_pair_between_two_open_panes_still_resizes() {
        let mut dock = pair_row();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        for tab in [Tab::Spectral, Tab::Tuning] {
            collapse(&mut dock, tab, true);
        }
        let mut window = settle(&mut folds, &mut dock, &mut dial, 1000.0);
        window = settle(&mut folds, &mut dock, &mut dial, window);
        let (first, last) = (NodeIndex(1), NodeIndex(6));
        let (before_first, before_last) = (width(&dock, first), width(&dock, last));
        drag(&mut dock, NodeIndex::root(), 40.0);
        let _ = frame(&mut folds, &mut dock, &mut dial, window);
        assert!(
            (width(&dock, first) - (before_first + 40.0)).abs() < 1.0,
            "the first pane moved {}",
            width(&dock, first) - before_first,
        );
        assert!(
            (width(&dock, last) - (before_last - 40.0)).abs() < 1.0,
            "the last pane moved {}",
            width(&dock, last) - before_last,
        );
    }

    #[test]
    fn two_rails_between_two_open_panes_still_resize() {
        let mut dock = row();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        for tab in [Tab::Spectral, Tab::Tuning] {
            collapse(&mut dock, tab, true);
        }
        let mut window = settle(&mut folds, &mut dock, &mut dial, 1000.0);
        window = settle(&mut folds, &mut dock, &mut dial, window);
        let (first, last) = (NodeIndex(1), NodeIndex(14));
        let (before_first, before_last) = (width(&dock, first), width(&dock, last));
        drag(&mut dock, NodeIndex::root(), 40.0);
        let _ = frame(&mut folds, &mut dock, &mut dial, window);
        assert!(
            (width(&dock, first) - (before_first + 40.0)).abs() < 1.0,
            "the first pane should have taken the 40 the drag gave it, and moved {}",
            width(&dock, first) - before_first,
        );
        assert!(
            (width(&dock, last) - (before_last - 40.0)).abs() < 1.0,
            "the last pane should have paid for it, and moved {}",
            width(&dock, last) - before_last,
        );
    }

    /// A separator with a fold below it divides two panes that are both on
    /// screen, and dragging it has to move them — the fold owns that fraction
    /// (it is what sends the folded width out to the window) and used to
    /// overwrite the drag on the frame after the one it was made on, so the
    /// handle lit up, the cursor changed, and nothing moved.
    #[test]
    fn a_separator_with_a_fold_below_it_still_resizes_what_it_divides() {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        collapse(&mut dock, Tab::Spectral, true);
        let window = settle(&mut folds, &mut dock, &mut dial, 1000.0);
        let (pictures, settings) = (width(&dock, PICTURES), width(&dock, SETTINGS));
        // The handle on the far side of the rail, which belongs to the split
        // above the fold: the picture pair on one side of it, the settings
        // column on the other, and both of them on screen.
        drag(&mut dock, NodeIndex::root(), 40.0);
        let asked = frame(&mut folds, &mut dock, &mut dial, window);
        assert!(
            (width(&dock, PICTURES) - (pictures + 40.0)).abs() < 1.0,
            "the pair should have taken the 40pt the drag gave it, and is {}",
            width(&dock, PICTURES) - pictures,
        );
        assert!(
            (width(&dock, SETTINGS) - (settings - 40.0)).abs() < 1.0,
            "the column on the other side of the handle should have paid for it",
        );
        assert!((width(&dock, SPECTRAL) - 26.0).abs() < 0.01, "the rail is still a rail");
        assert!((asked - window).abs() < 0.01, "and a drag is not a resize: the window stays");
        // Held, rather than undone one frame later, which is the whole bug.
        let _ = frame(&mut folds, &mut dock, &mut dial, window);
        assert!(
            (width(&dock, PICTURES) - (pictures + 40.0)).abs() < 1.0,
            "the frame after should be where the drag left it",
        );
    }

    /// A drag while a pane is folded is a drag on the LAYOUT, not on the
    /// rendering of it: unfolding hands the pane back into the arrangement the
    /// drag left, rather than into the one it was folded from.
    #[test]
    fn a_drag_taken_while_a_pane_is_folded_survives_the_unfold() {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        collapse(&mut dock, Tab::Spectral, true);
        let window = settle(&mut folds, &mut dock, &mut dial, 1000.0);
        drag(&mut dock, NodeIndex::root(), 40.0);
        let window = frame(&mut folds, &mut dock, &mut dial, window);
        let (lattice, settings) = (width(&dock, LATTICE), width(&dock, SETTINGS));
        collapse(&mut dock, Tab::Spectral, false);
        let _ = settle(&mut folds, &mut dock, &mut dial, window);
        assert!(
            (width(&dock, SETTINGS) - settings).abs() < 1.0,
            "the column stays where the drag put it: the analyzer comes back out of the window",
        );
        assert!(
            (width(&dock, LATTICE) - lattice).abs() < 1.0,
            "and so does the lattice, which the drag also moved",
        );
    }

    /// egui_dock re-clamps every separator's fraction on every frame, dragged or
    /// not, to keep `separator.extra` points of pane on either side of it. A rail
    /// is far narrower than that, so a folded layout's own fractions come back
    /// off the tree clamped — which the layout never sees, and which reads
    /// exactly like a drag. Read as one, every frame a pane spends folded
    /// re-dials the whole layout, and the panes walk.
    #[test]
    fn the_clamp_egui_dock_applies_to_every_separator_is_not_a_drag() {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        reclamp(&mut dock);
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        // Both pictures, so the root folds a whole subtree into two rails and
        // the fraction it writes is nowhere near what the clamp allows.
        collapse(&mut dock, Tab::Lattice, true);
        let mut window = settle(&mut folds, &mut dock, &mut dial, 1000.0);
        collapse(&mut dock, Tab::Spectral, true);
        window = settle(&mut folds, &mut dock, &mut dial, window);
        let before = [LATTICE, SPECTRAL, SETTINGS].map(|node| width(&dock, node));
        for _ in 0..6 {
            reclamp(&mut dock);
            window = frame(&mut folds, &mut dock, &mut dial, window);
        }
        for (node, was) in [LATTICE, SPECTRAL, SETTINGS].iter().zip(before) {
            let now = width(&dock, *node);
            assert!((now - was).abs() < 0.5, "{node:?} walked from {was} to {now}");
        }
    }

    /// Pulling a rail out is the fold run backwards at a width the user chose:
    /// the pane comes back that wide, the window pays what the fold took, and
    /// the pane's own sibling pays the difference between the two — which is
    /// what a separator dragged between two panes always costs.
    #[test]
    fn a_rail_pulled_open_comes_back_at_the_width_it_was_pulled_to() {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        let (lattice, analyzer) = (width(&dock, LATTICE), width(&dock, SPECTRAL));
        collapse(&mut dock, Tab::Spectral, true);
        let folded = settle(&mut folds, &mut dock, &mut dial, 1000.0);
        pull(&mut dial, PICTURES, Side::Right, SPECTRAL, 300.0);
        let window = settle(&mut folds, &mut dock, &mut dial, folded);
        assert!(
            !collapsed_tab(&dock, Tab::Spectral),
            "a rail pulled out past its own width is a pane again",
        );
        assert!(
            (width(&dock, SPECTRAL) - 300.0).abs() < 1.0,
            "it should come back at the 300 it was pulled to, not the {} it folded from",
            width(&dock, SPECTRAL),
        );
        assert!(
            (window - 1000.0).abs() < 1.0,
            "the window pays back what the fold took it down by, and no more: {window}",
        );
        assert!(
            (width(&dock, LATTICE) - (lattice - (300.0 - analyzer))).abs() < 1.0,
            "the pane it was pulled out over pays the rest: {} from {lattice}",
            width(&dock, LATTICE),
        );
        // Outside the split nothing moves, which is what a pull has in common
        // with the arrow: neither is a re-layout of the panes beside it.
        assert!((width(&dock, SETTINGS) - 298.0).abs() < 1.0, "the column keeps its width");
    }

    /// A pull on one pane of a folded PAIR brings that pane back and leaves the
    /// other where it was: a rail can hold panes that were folded separately, and
    /// a pull is the pane's own arrow with a width on it, not the subtree's.
    #[test]
    fn a_pull_on_a_folded_pair_opens_the_pane_it_was_aimed_at() {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        for tab in [Tab::Lattice, Tab::Spectral] {
            collapse(&mut dock, tab, true);
        }
        let folded = settle(&mut folds, &mut dock, &mut dial, 1000.0);
        pull(&mut dial, NodeIndex::root(), Side::Left, LATTICE, 500.0);
        let _ = settle(&mut folds, &mut dock, &mut dial, folded);
        assert!(!collapsed_tab(&dock, Tab::Lattice), "the lattice was the one pulled out");
        assert!(collapsed_tab(&dock, Tab::Spectral), "the analyzer was not, so it is still a rail");
        assert!(
            (width(&dock, PICTURES) - 500.0).abs() < 1.5,
            "the pair should come back at the 500 it was pulled to, and is {}",
            width(&dock, PICTURES),
        );
        // Which the still-folded analyzer takes a rail's worth of.
        assert!(
            (width(&dock, LATTICE) - (500.0 - 26.0 - 4.0)).abs() < 1.5,
            "the lattice should have all of it but the analyzer's rail, and has {}",
            width(&dock, LATTICE),
        );
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
    /// The same ceiling, against a separator wiggled rather than a window
    /// dragged. A drag raises what an unfold may ask for by the width it dialled,
    /// which is width the user asked for — but a wiggle asks for nothing on the
    /// way back, so a ceiling that only ever rose would buy a wider window every
    /// time the separator went out, and hand it over the next time a fold was
    /// released in a window that had been dragged out under it.
    #[test]
    fn wiggling_a_separator_while_folded_does_not_raise_that_ceiling() {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut window = Window::new(1500.0);
        window.settle(&mut folds, &mut dock);
        collapse(&mut dock, Tab::Lattice, true);
        window.settle(&mut folds, &mut dock);
        // Ten drags that cancel out, each settled the way a pointer moving over
        // a separator settles them: one frame apiece.
        for _ in 0..5 {
            for delta in [80.0, -80.0] {
                drag(&mut dock, NodeIndex::root(), delta);
                window.frame(&mut folds, &mut dock);
            }
        }
        // Dragged back out while the pane is still a rail, which is what makes
        // the layout want a window wider than the display has ever been.
        window.size = 1500.0;
        window.area = 1500.0;
        window.settle(&mut folds, &mut dock);
        collapse(&mut dock, Tab::Lattice, false);
        window.settle(&mut folds, &mut dock);
        assert!(
            window.size <= 1501.0,
            "the wiggles bought {} of window against one that has never been wider than 1500",
            window.size,
        );
    }

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
