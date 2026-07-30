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
//! ## The layout is points; the rails are a rendering of it
//!
//! The layout is a width per pane, in points, held in [`Dial`] and changed only
//! by something the user did. What is on SCREEN is derived from it each frame:
//! the folded subtrees squeezed to rails, and whatever is left shared out among
//! the panes in proportion to what they are dialled at ([`Points::spread`]).
//!
//! Points rather than fractions, because a fraction is a share of a window and
//! therefore says something different at every window size. A folded pane
//! holding a fraction goes on growing with a window it is not being drawn in,
//! and hands back more than it took — so opening a pane makes the panes beside
//! it smaller, which is a re-layout nobody asked for. A folded pane holding
//! POINTS is simply spared: a window resize is worn by the panes on screen to
//! wear it ([`Points::refit`]), and unfolding is the rail giving up standing in
//! for a width that never moved. Nothing is restored on the way out, so nothing
//! can drift on the way out.
//!
//! That makes a fold reversible by construction rather than by bookkeeping.
//! Folds compose, because a folded subtree is one term in the same sum. A
//! resize the window refuses costs nothing, since the points are not what was
//! drawn. `every_round_trip_of_clicks_lands_where_it_started` is the test that
//! holds all of it: every sequence of up to six arrow clicks that ends where it
//! started, at three window sizes.
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
//! rewrites each of them every frame — so a separator dragged across a folded
//! layout would snap back one frame after the handle had lit up and the cursor
//! had changed for it. What the drag lands on is a fraction this pass wrote, so
//! the difference from what was written IS the drag, and multiplying it by the
//! width that split came out at turns it back into points ([`drags`]). No
//! inverse of the derivation is needed for it: the layout and the thing the
//! user pointed at are both widths on screen.
//!
//! Only what is on screen can trade. A rail is a fixed number of points
//! whichever side of the boundary it sits, and a folded pane is holding the
//! width it comes back at rather than any width the drag can see — so the drag
//! moves the visible panes on each side and leaves the rest exactly where it is.
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

/// What each sideways fold is holding — the one thing about a folded layout
/// that cannot be read back off the tree: the width the window owes the pane
/// when it opens again.
///
/// The layout itself is points, one per pane, and it lives in [`Dial`]. Nothing
/// here is a fraction. A fraction is a share of a window, so it says something
/// different at every window size — a folded pane's share goes on growing with
/// a window it is not being drawn in, and hands back more than it took. Points
/// say the same thing at any window, so a fold is reversible by construction
/// and a resize is the visible panes' business alone.
///
/// Persisted with the dock (see `UiPersist`), because the points are not: a
/// layout is loaded into whatever window it finds, and its widths are seeded
/// from the fractions in the tree — where a folded pane's fraction is a rail's.
/// This is what tells the seed how wide that pane was before it folded.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Folds(Vec<Fold>);

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Fold {
    /// Which split, as indices into the dock. Valid only while the tree keeps
    /// its shape — the entry is dropped as soon as the node stops being folded,
    /// which covers re-docking.
    surface: usize,
    node: usize,
    /// Which child is the folded one. The collapsed flags say so while the fold
    /// stands; this is for the frame AFTER the arrow is clicked, where they no
    /// longer do and the window still has to be paid back.
    #[serde(default)]
    side: Side,
    /// The points the folded subtree was drawn at when it folded, which is what
    /// folding took off the window and what unfolding gives back.
    ///
    /// Only the SEED reads it, and only after a load: while the editor is
    /// running, the pane's points are simply left where they were and the rail
    /// stands in for them, so there is nothing to restore.
    ///
    /// Zero in a blob written before it was recorded, where `fraction` is what
    /// there is instead.
    #[serde(default)]
    width: f32,
    /// The dialled FRACTION a blob written before the layout was points is
    /// holding, and all it holds. The seed turns it into points, which is what
    /// a layout is now; nothing else reads it and nothing writes it.
    ///
    /// Kept because the wire format has to go on parsing: `UiPersist` is one
    /// RON document, so a blob this cannot read costs the whole saved layout,
    /// not just its folds.
    #[serde(default)]
    fraction: f32,
    /// The window this fold was taken at, which is the window unfolding it owes
    /// back. [`Dial`] is runtime-only, so without this a project reopened with a
    /// pane already folded has no record of how wide the window has been — and
    /// the unfold's growth cap reads exactly that record, so an empty one holds
    /// the window at the width the fold left it.
    ///
    /// Zero in a blob written before it was recorded. There is no history to
    /// recover there, so the cap falls back to the window on screen, which is
    /// what it did for every blob then.
    #[serde(default)]
    window: f32,
}

/// Which child of a split is the folded one.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
enum Side {
    #[default]
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

/// A rail the user has pulled out into a pane again, and how wide they pulled
/// it (see [`grab`]).
///
/// Carried from the pull to the next frame rather than acted on where it
/// happens, because [`Folds::apply`] is where the layout is, and a width that
/// is not measured against the layout is not a width anything can be dialled to.
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
    /// Bring the layout up to date with what the user did to it, and write the
    /// fractions that draw it in the window there is.
    ///
    /// Runs BEFORE the dock lays out, because a fraction is layout's input.
    ///
    /// The layout is [`Dial::points`]: a width per pane, held between frames and
    /// changed only by something the user did. Three things can have happened
    /// since the last pass, and each is a change to those widths:
    ///
    /// - a separator dragged, which moves points across a boundary ([`drags`]);
    /// - a pane folded or unfolded, which is the window's business rather than
    ///   the layout's — a folded pane KEEPS its points and is drawn as a rail
    ///   standing in for them, so unfolding restores nothing and cannot drift;
    /// - the window dragged, which shares itself out over the panes that are
    ///   visible ([`refit`]) and leaves the folded ones alone. That is the whole
    ///   of "a resize does not scale a collapsed pane".
    ///
    /// What is DRAWN is then those points fitted to `area` ([`spread`]): the
    /// rails at a fixed number of points, whatever is left shared out among the
    /// panes in proportion to what they are dialled at. When the window is the
    /// one the layout wants, that is the layout exactly; when it is not — a host
    /// that refused the fold's resize, or the floor it will not go under — the
    /// panes take the difference and the rails do not, which is the only place
    /// it can come from.
    ///
    /// Returns the points the window has to gain (negative: lose), which the
    /// shell asks its host for (see `SharedState::take_window_width_change`).
    ///
    /// `floor` is the narrowest the shell will let the window become. At the
    /// floor the window is no longer a free variable, so the layout stops
    /// following it: without that, a fold the window cannot pay for in full
    /// would re-fit to the window it got rather than the one it asked for, and
    /// unfolding would hand back the difference.
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
        // [`grab`]), which is where the width it was pulled to becomes layout.
        let pull = dial.pull.take();
        let gesturing = dial.gesturing;
        dial.panes.resize(dock.surfaces_count(), Points::default());
        for index in 0..dock.surfaces_count() {
            let surface = SurfaceIndex(index);
            let Some(tree) = dock.get_surface_mut(surface).and_then(Surface::node_tree_mut) else {
                continue;
            };
            if tree.is_empty() {
                continue;
            }
            reached.push(index);
            let main = surface == SurfaceIndex::main();
            // A floating dock window is laid out in its own window rather than
            // in the dock area, and its size is not ours to know — so that root
            // is measured, a frame stale, which is all a fold there needs: it
            // moves no window, so its layout simply re-fits to what it is in.
            let area = if main {
                area
            } else {
                tree[NodeIndex::root()].rect().map_or(f32::NAN, |rect| rect.width())
            };
            if !area.is_finite() || area <= 0.0 {
                continue;
            }
            let holds = holds(tree);
            let points = &mut dial.panes[index];
            // A tree this pass has never seen: a fresh dock, a load, or one the
            // user has re-docked. The fractions in it are the only record of
            // what the panes are dialled to, so that is the seed — with the
            // folded ones put back to the width they were persisted at, since
            // their fraction in the tree is a rail's.
            if points.at.len() != tree.len() {
                points.seed(tree, area, separator);
                for fold in self.0.iter().filter(|fold| fold.surface == index) {
                    if fold.node >= tree.len() {
                        continue;
                    }
                    let node = NodeIndex(fold.node);
                    if fold.width > 0.0 {
                        points.scale(tree, fold.side.of(node), fold.width);
                    } else if fold.fraction > 0.0 {
                        points.divide(tree, node, fold.fraction, separator);
                    }
                }
            }
            // A separator the user dragged lands on a fraction this pass wrote,
            // so it arrives as a difference from what was written — points
            // moving across a boundary, which is all a drag ever means.
            let (want, fixed) = wants(tree, &holds, &points.at, rail, separator);
            drags(tree, points, &holds, &want, &fixed, style, gesturing);
            // The pull opens its pane first, so the width it was pulled to is
            // priced against the layout that will hold it.
            let mine = pull.filter(|pull| {
                pull.surface == index
                    && folded_side(tree, NodeIndex(pull.node)) == Some(pull.side)
                    && collapsed(tree, NodeIndex(pull.leaf))
            });
            if let Some(pull) = &mine {
                let split = NodeIndex(pull.node);
                // What the whole rail was worth before it folded, which is what
                // the pull is measured against: the width names the subtree on
                // screen, and a rail can hold panes that stay folded.
                let was = self
                    .0
                    .iter()
                    .find(|fold| fold.is(surface, split))
                    .map_or(0.0, |fold| fold.width);
                uncollapse(tree, NodeIndex(pull.leaf));
                let holds = self::holds(tree);
                let (_, fixed) = wants(tree, &holds, &points.at, rail, separator);
                let child = pull.side.of(split);
                // Whatever inside the rail is still a rail keeps its points, so
                // the pane that opened makes up the rest of the width pulled to.
                points.share(tree, &holds, child, pull.width - fixed[child.0]);
                // The window pays the difference between the rail and the pane
                // it stood in for, as it does for the arrow; the panes beside it
                // inside the split pay the rest, exactly as they would have had
                // the pane been dragged wider once it was a pane again. Asking
                // the window for the whole of a pull would let a gesture inside
                // one split grow the editor without limit.
                let beside = match pull.side {
                    Side::Left => Side::Right,
                    Side::Right => Side::Left,
                }
                .of(split);
                let spare = points.visible(tree, &holds, beside);
                points.share(tree, &holds, beside, spare - (pull.width - was));
            }
            // Folds are re-read after the pull, which has just cleared flags.
            let holds = if mine.is_some() { self::holds(tree) } else { holds };
            let moved = self.reconcile(tree, surface, &holds, points, area) || mine.is_some();
            // The widest this window has been, for a session that was not there
            // to watch it get that wide: the folds came off the persist blob,
            // and each one remembers the window it was taken at.
            if dial.widest <= 0.0 {
                dial.widest = self
                    .0
                    .iter()
                    .filter(|fold| fold.surface == index)
                    .fold(0.0_f32, |widest, fold| widest.max(fold.window));
            }
            let (want, fixed) = wants(tree, &holds, &points.at, rail, separator);
            let settled = (area - dial.area).abs() < 0.01;
            // The window moved without this pass asking it to — the user
            // dragged it, or the host resized us — so the layout follows, over
            // the panes that are on screen to wear it.
            let follow = !settled && !dial.asked;
            // Asked and did not move: a host that refused, and the layout is
            // wanting a window that is not coming. Take what there is instead.
            // Not at the floor, where "the window did not move" is what the
            // floor MEANS rather than a refusal — re-fitting there would bank
            // the difference between what a fold asked for and what it got as
            // though the user had widened the window, and hand it back on the
            // way out.
            let refused = settled && dial.asked && !moved;
            let mut want = want;
            if (follow || refused) && area > floor + 1.0 && dial.area > 0.0 {
                points.refit(&want, &fixed, area);
                (want, _) = wants(tree, &holds, &points.at, rail, separator);
            }
            if main {
                // What the layout would want with every pane OPEN, which is what
                // the reset button hands back: it replaces the dock with one that
                // has no folds in it, and has no frame to derive that in.
                let (open, _) = wants(tree, &[], &points.at, rail, separator);
                dial.wants = open[0];
            }
            dial.area = area;
            // Only a fold or an unfold moves the window. Any other gap between
            // the layout and the window is one the window is not answering for
            // — a host that refused, or a floor it will not go under — and
            // asking again every frame would be an argument, not a request.
            if main && moved {
                // Never past the widest this window has actually been. Fold a
                // pane and drag the window back out, and every visible pane has
                // grown into the width the fold freed — so unfolding asks for
                // that width on top of a window that already spent it, which is
                // how a plugin window ends up wider than the display it is on.
                // Shrinking is never capped; only the growth is.
                dial.widest = dial.widest.max(area);
                ask += (want[0] - area).min((dial.widest - area).max(0.0));
            }
            dial.asked = main && moved;
            // A fold is a two-step, and the step it is NOT is this one. The
            // window is still the one being left — the resize has been asked
            // for and not yet answered — and every arrangement that fits the
            // old window is a lie about where the panes are going. Drawn
            // settled, they stretch by the ratio between the two windows;
            // drawn fitted, the folded pane's neighbour swells to take the
            // freed width and gives it back a frame later. Both read as a
            // flicker for the sake of one frame, so this frame draws what it
            // drew last frame and the layout changes on the frame that has the
            // window it was computed for.
            //
            // Only where there IS a window to wait for. A floating dock window
            // never asks for one, so deferring there would be a frame of
            // nothing followed by a frame of nothing.
            if !(moved && main) {
                points.spread(tree, &want, &fixed, area, separator);
            }
        }
        // Entries naming a surface the dock no longer has.
        self.0.retain(|fold| reached.contains(&fold.surface));
        ask
    }

    /// Take an entry for every split that has just folded, and give one up for
    /// every split that has just unfolded. Answers whether either happened,
    /// which is what decides if the window has to move.
    ///
    /// Nothing is written into the layout either way: a folded pane keeps the
    /// points it had open the whole time it is drawn as a rail. The entry is
    /// what the WINDOW is owed, and what a reload needs to seed those points
    /// from.
    fn reconcile(
        &mut self,
        tree: &Tree<Tab>,
        surface: SurfaceIndex,
        holds: &[Hold],
        points: &Points,
        area: f32,
    ) -> bool {
        let mut moved = false;
        // Unfolded, or re-docked out from under the entry.
        self.0.retain(|fold| {
            if fold.surface != surface.0 {
                return true;
            }
            let held = holds.get(fold.node).and_then(|hold| hold.side) == Some(fold.side);
            moved |= !held;
            held
        });
        for (index, hold) in holds.iter().enumerate() {
            let Some(side) = hold.side else { continue };
            if self.0.iter().any(|fold| fold.is(surface, NodeIndex(index))) {
                continue;
            }
            // First frame of this fold. `area` is still the pre-fold window —
            // the resize has been decided and not yet asked for — which is the
            // width this fold is about to take off it.
            self.0.push(Fold {
                surface: surface.0,
                node: index,
                side,
                width: points.span(tree, side.of(NodeIndex(index))),
                fraction: 0.0,
                window: area,
            });
            moved = true;
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
    /// Panel pane's "Reset layout").
    ///
    /// Returns the points the window is owed for them. The layout that replaces
    /// this one has every pane open, so it wants the whole width the folds were
    /// keeping off the window.
    #[must_use]
    pub fn clear(&mut self, dial: &Dial, area: f32) -> f32 {
        self.forget();
        // Held to the same ceiling as the rail's arrow (see the ask in
        // [`Folds::apply`]), because it undoes the same fold and therefore owes
        // the same width. A layout wanting a window that never arrived — a host
        // that refused, or a drag back out while folded — prices itself well
        // above anything the window has been, and a button that hands that
        // price to the host is how the editor ends up wider than the display.
        let want = if dial.wants > area { dial.wants - area } else { 0.0 };
        want.min((dial.widest - area).max(0.0))
    }

    /// Whether anything is being remembered. Nothing in the draw needs this —
    /// it is how a test says "this dock was replaced, so the folds that named
    /// its splits are gone too".
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

/// What the shell carries between frames: the layout in points, and what the
/// window is doing to it.
///
/// Runtime-only. A layout loaded into a window it was not saved at is seeded
/// from the fractions it finds there, which is the same thing that happens when
/// the dock is re-docked.
#[derive(Default)]
pub struct Dial {
    /// One per surface of the dock.
    panes: Vec<Points>,
    /// The window the main surface's layout wants, for the reset button, which
    /// undoes folds without a frame to derive anything in.
    wants: f32,
    /// The area the last pass laid out in, so a window that moved on its own
    /// can be told from one that is answering an ask of this pass's.
    area: f32,
    /// Whether the last frame asked the window to change. If the window has not
    /// moved by the next one, the ask was refused and the layout takes what it
    /// has.
    asked: bool,
    /// The widest this window has actually been, which is as far as an unfold
    /// may ask it to grow. See the ask in [`Folds::apply`].
    widest: f32,
    /// Whether a pointer was down (or has just come up) when this frame's input
    /// was read. Only a gesture moves a fraction on purpose: egui_dock re-clamps
    /// every separator on every frame, dragged or not, so without this a window
    /// narrow enough for that clamp to bite reads as a drag every frame and
    /// walks the layout toward 50/50 by itself (see [`drags`]).
    gesturing: bool,
    /// A rail pulled open, waiting for the frame that can price it. Set by
    /// [`paint`], which is where the pull is let go of, and taken by
    /// [`Folds::apply`], which is where a width becomes layout.
    pull: Option<Pull>,
}

/// One surface's layout, in points.
#[derive(Clone, Default)]
struct Points {
    /// Per node, the width it is dialled to. Only the LEAVES are held here; a
    /// split's width is the sum of what is under it, worked out afresh each
    /// frame ([`wants`]), because a split is not a thing the user dials.
    ///
    /// A folded pane keeps the width it had open the whole time it is drawn as
    /// a rail. That is the whole of unfolding — the rail stops standing in for
    /// it — and it is why a window resize has to skip it ([`Points::refit`]).
    at: Vec<f32>,
    /// The fraction this pass last wrote into each split, and the width that
    /// split came out at on screen. A fraction that has moved off `wrote` is a
    /// separator the user dragged, and `drew` is what turns the difference back
    /// into points.
    wrote: Vec<f32>,
    drew: Vec<f32>,
}

impl Dial {
    /// What the shell saw of the pointer this frame, before [`Folds::apply`]
    /// reads the fractions the last one left behind.
    pub fn watch_pointer(&mut self, gesturing: bool) {
        self.gesturing = gesturing;
    }
}

impl Points {
    /// Take the layout off the fractions in the tree, for a tree this pass has
    /// not seen before. What is in them is what is on screen, so what comes out
    /// is the layout as drawn — folded panes at a rail's width, which is why
    /// [`Folds::apply`] puts the persisted widths back over the top.
    fn seed(&mut self, tree: &Tree<Tab>, area: f32, separator: f32) {
        self.at = vec![0.0; tree.len()];
        self.wrote = vec![f32::NAN; tree.len()];
        self.drew = vec![0.0; tree.len()];
        self.drew[0] = area;
        for index in 0..tree.len() {
            let node = NodeIndex(index);
            let (left, right) = (node.left(), node.right());
            let width = self.drew[index];
            match &tree[node] {
                Node::Leaf(_) => self.at[index] = width,
                _ if right.0 >= tree.len() => {}
                Node::Vertical(_) => {
                    (self.drew[left.0], self.drew[right.0]) = (width, width);
                }
                Node::Horizontal(split) => {
                    let before = (width * split.fraction - separator * 0.5).max(0.0);
                    self.drew[left.0] = before;
                    self.drew[right.0] = (width - separator - before).max(0.0);
                }
                Node::Empty => {}
            }
        }
    }

    /// Scale every pane under `node` so the subtree comes out `width` points
    /// wide — how a width names a layout, whether it came off a pull, a drag or
    /// a persist blob.
    fn scale(&mut self, tree: &Tree<Tab>, node: NodeIndex, width: f32) {
        let was = self.span(tree, node);
        if was <= 0.0 || !width.is_finite() || width <= 0.0 {
            return;
        }
        self.stretch(tree, node, width / was);
    }

    /// Put a split's two sides back to a fraction that was dialled before the
    /// layout was points — the one thing a blob from then holds.
    fn divide(&mut self, tree: &Tree<Tab>, node: NodeIndex, fraction: f32, separator: f32) {
        let whole = self.drew.get(node.0).copied().unwrap_or(0.0);
        if whole <= 0.0 {
            return;
        }
        let before = (whole * fraction - separator * 0.5).max(0.0);
        self.scale(tree, node.left(), before);
        self.scale(tree, node.right(), (whole - separator - before).max(0.0));
    }

    /// The points a subtree is dialled to, side by side: a vertical split's two
    /// children are the same width rather than two widths, so this is not the
    /// sum of the leaves under it.
    fn span(&self, tree: &Tree<Tab>, node: NodeIndex) -> f32 {
        let (left, right) = (node.left(), node.right());
        match &tree[node] {
            Node::Leaf(_) => self.at.get(node.0).copied().unwrap_or(0.0),
            _ if right.0 >= tree.len() => 0.0,
            Node::Vertical(_) => self.span(tree, left).max(self.span(tree, right)),
            Node::Horizontal(_) => self.span(tree, left) + self.span(tree, right),
            Node::Empty => 0.0,
        }
    }

    /// Scale the panes ON SCREEN under `node` so they come to `width` points
    /// between them, leaving anything folded exactly where it is — which is
    /// what makes a drag, like a resize, the visible panes' business alone.
    fn share(&mut self, tree: &Tree<Tab>, holds: &[Hold], node: NodeIndex, width: f32) {
        let was = self.visible(tree, holds, node);
        if was <= 0.0 || !width.is_finite() || width <= 0.0 {
            return;
        }
        self.stretch_visible(tree, holds, node, width / was);
    }

    /// The points a subtree's visible panes are dialled to, side by side.
    fn visible(&self, tree: &Tree<Tab>, holds: &[Hold], node: NodeIndex) -> f32 {
        if holds.get(node.0).is_some_and(|hold| hold.inside) {
            return 0.0;
        }
        let (left, right) = (node.left(), node.right());
        match &tree[node] {
            Node::Leaf(_) => self.at.get(node.0).copied().unwrap_or(0.0),
            _ if right.0 >= tree.len() => 0.0,
            Node::Vertical(_) => {
                self.visible(tree, holds, left).max(self.visible(tree, holds, right))
            }
            Node::Horizontal(_) => {
                self.visible(tree, holds, left) + self.visible(tree, holds, right)
            }
            Node::Empty => 0.0,
        }
    }

    fn stretch_visible(&mut self, tree: &Tree<Tab>, holds: &[Hold], node: NodeIndex, by: f32) {
        if holds.get(node.0).is_some_and(|hold| hold.inside) {
            return;
        }
        let (left, right) = (node.left(), node.right());
        match &tree[node] {
            Node::Leaf(_) => {
                if let Some(at) = self.at.get_mut(node.0) {
                    *at *= by;
                }
            }
            _ if right.0 >= tree.len() => {}
            _ => {
                self.stretch_visible(tree, holds, left, by);
                self.stretch_visible(tree, holds, right, by);
            }
        }
    }

    fn stretch(&mut self, tree: &Tree<Tab>, node: NodeIndex, by: f32) {
        let (left, right) = (node.left(), node.right());
        match &tree[node] {
            Node::Leaf(_) => {
                if let Some(at) = self.at.get_mut(node.0) {
                    *at *= by;
                }
            }
            _ if right.0 >= tree.len() => {}
            _ => {
                self.stretch(tree, left, by);
                self.stretch(tree, right, by);
            }
        }
    }

    /// Share a window the user moved out over the panes that are on screen to
    /// wear it, and leave the folded ones exactly where they are.
    ///
    /// A rail is a fixed number of points and a folded pane is not being drawn
    /// at all, so neither has any business growing with the window: the pane
    /// would come back wider than it went away, and the rail would come back as
    /// a stripe. What scales is `want - fixed`, which is the panes.
    fn refit(&mut self, want: &[f32], fixed: &[f32], area: f32) {
        let (whole, rails) = (want[0], fixed[0]);
        let scaling = whole - rails;
        if scaling <= 0.0 || area <= rails {
            return;
        }
        let by = (area - rails) / scaling;
        for (index, at) in self.at.iter_mut().enumerate() {
            // Fixed nodes are the folded subtrees, whose panes hold the width
            // they will come back at.
            if fixed[index] <= 0.0 {
                *at *= by;
            }
        }
    }

    /// Write the fraction that draws this layout in the window there IS: the
    /// rails at a fixed number of points, and whatever is left shared out among
    /// the panes in proportion to what they are dialled at.
    ///
    /// Where the window is the one the layout wants, that is the layout exactly.
    /// Where it is not — a host that refused a fold's resize, or the floor —
    /// the panes wear the difference and the rails do not, which is the only
    /// place it can come from. `at` does not move for it, so unfolding still
    /// hands back exactly what folding took.
    fn spread(
        &mut self,
        tree: &mut Tree<Tab>,
        want: &[f32],
        fixed: &[f32],
        area: f32,
        separator: f32,
    ) {
        self.drew[0] = area;
        for index in 0..tree.len() {
            let node = NodeIndex(index);
            let (left, right) = (node.left(), node.right());
            if right.0 >= tree.len() || !tree[node].is_parent() {
                continue;
            }
            let width = self.drew[index];
            if tree[node].is_horizontal() {
                let (fl, fr) = (fixed[left.0], fixed[right.0]);
                let (sl, sr) = (want[left.0] - fl, want[right.0] - fr);
                let slack = width - separator - fl - fr;
                let by = if sl + sr > 0.0 { (slack / (sl + sr)).max(0.0) } else { 0.0 };
                let before = (fl + sl * by).clamp(0.0, (width - separator).max(0.0));
                self.drew[left.0] = before;
                self.drew[right.0] = (width - separator - before).max(0.0);
                if width > 0.0 {
                    let fraction = ((before + separator * 0.5) / width).clamp(0.0, 1.0);
                    set_fraction(tree, node, fraction);
                    self.wrote[index] = fraction;
                }
            } else {
                (self.drew[left.0], self.drew[right.0]) = (width, width);
            }
        }
    }
}

/// What each node is asking for on screen, and how much of that will not scale.
///
/// A folded subtree asks for the rails it renders as, which is a fixed number of
/// points however wide the window gets; everything else asks for the points its
/// panes are dialled to. Bottom-up, so a split is what is under it.
///
/// `fixed` is the part of `want` that a window resize cannot touch — the rails,
/// and the separators between them — which is what [`Points::refit`] and
/// [`Points::spread`] divide the window against.
fn wants(
    tree: &Tree<Tab>,
    holds: &[Hold],
    at: &[f32],
    rail: f32,
    separator: f32,
) -> (Vec<f32>, Vec<f32>) {
    let mut want = vec![0.0; tree.len()];
    let mut fixed = vec![0.0; tree.len()];
    for index in (0..tree.len()).rev() {
        let node = NodeIndex(index);
        let (left, right) = (node.left(), node.right());
        // Inside a fold: drawn as the rails it holds, and everything under it
        // is chrome. The panes keep their points regardless — they are what the
        // subtree comes back at — but nothing on screen is a share of them.
        if holds.get(index).is_some_and(|hold| hold.inside) {
            want[index] = rail_span(rail_columns(tree, node), rail, separator);
            fixed[index] = want[index];
            continue;
        }
        (want[index], fixed[index]) = match &tree[node] {
            Node::Leaf(_) => (at.get(index).copied().unwrap_or(0.0), 0.0),
            _ if right.0 >= tree.len() => (0.0, 0.0),
            Node::Vertical(_) => (
                want[left.0].max(want[right.0]),
                fixed[left.0].max(fixed[right.0]),
            ),
            Node::Horizontal(_) => (
                want[left.0] + separator + want[right.0],
                fixed[left.0] + separator + fixed[right.0],
            ),
            Node::Empty => (0.0, 0.0),
        };
    }
    // The window itself is not a rail, so a layout with no fold in it has
    // nothing fixed about it at all.
    if fixed[0] <= separator * tree.len() as f32 && !holds.iter().any(|hold| hold.side.is_some()) {
        fixed = vec![0.0; tree.len()];
    }
    (want, fixed)
}

/// Read every separator the user dragged since the last pass back into the
/// layout: points moving across a boundary, which is all a drag ever means.
///
/// The drag lands on a fraction this pass wrote, one frame after the rectangles
/// it was measured against were drawn — so the difference from what was written,
/// times the width that split came out at, is the points the boundary travelled.
/// No inverse is needed for it, and no bisection: the layout and the thing the
/// user pointed at are in the same units.
///
/// Only while a pointer is doing something, and only past what egui_dock's own
/// per-frame clamp explains. It re-clamps every separator's fraction on every
/// frame, dragged or not, to keep `separator.extra` points of pane on either
/// side — so the fractions this pass writes come back clamped, and a window
/// narrow enough for the clamp to bite would otherwise read as a drag every
/// frame and walk the layout toward 50/50 by itself. The rectangles the real
/// dock clamps against are rounded to whole pixels, which is why [`unmoved`]
/// alone is not enough to tell the two apart.
fn drags(
    tree: &Tree<Tab>,
    points: &mut Points,
    holds: &[Hold],
    want: &[f32],
    fixed: &[f32],
    style: &egui_dock::Style,
    gesturing: bool,
) {
    if !gesturing {
        return;
    }
    let separator = style.separator.width;
    let floor = style.separator.extra - separator * 0.5;
    let mins = min_widths(tree, floor, style.tab_bar.height, separator);
    let drawn = points.drew.clone();
    for index in 0..tree.len() {
        let node = NodeIndex(index);
        let (left, right) = (node.left(), node.right());
        if right.0 >= tree.len() || !tree[node].is_horizontal() {
            continue;
        }
        let (Node::Horizontal(split), Some(wrote)) =
            (&tree[node], points.wrote.get(index).copied())
        else {
            continue;
        };
        let drew = drawn.get(index).copied().unwrap_or(0.0);
        if !wrote.is_finite() || drew <= 0.0 {
            continue;
        }
        let moved = split.fraction - wrote;
        if moved.abs() < 1e-6 {
            continue;
        }
        if (split.fraction - unmoved(wrote, drew, style.separator.extra)).abs() < 1e-6 {
            continue;
        }
        // Only what is on SCREEN can trade: a rail is a fixed number of points
        // whichever side of the boundary it is on, and a folded pane is holding
        // the width it comes back at rather than any width the drag can see.
        let (before, after) = (want[left.0] - fixed[left.0], want[right.0] - fixed[right.0]);
        if before <= 0.0 || after <= 0.0 {
            continue;
        }
        // The drag is in DRAWN points, and the layout is in dialled ones. They
        // are the same thing at the window the layout wants, and a fixed ratio
        // apart at the one a host refused it (see [`Points::spread`]).
        let (dialled, whole) = (want[0] - fixed[0], drawn[0] - fixed[0]);
        if dialled <= 0.0 || whole <= 0.0 {
            continue;
        }
        // Held to what the panes on either side need, which egui_dock's own
        // limit does not know: it holds the two CHILDREN of the split apart and
        // nothing deeper in either of them (see [`min_widths`]).
        let room = |span: f32, min: f32| (span - min).max(0.0);
        let (spare, take) = (
            room(before, mins[left.0] - fixed[left.0]),
            room(after, mins[right.0] - fixed[right.0]),
        );
        let delta = (moved * drew * dialled / whole).clamp(-spare, take);
        if delta.abs() < 1e-4 {
            continue;
        }
        points.share(tree, holds, left, before + delta);
        points.share(tree, holds, right, after - delta);
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
