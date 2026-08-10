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
//! edge of the window, so [`paint`](fn@paint) draws the rail as the pane's own tab, name
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
//! `Workspace::take_window_width_change`).
//!
//! Everything this module reads across frames — the tree, the [`Folds`], the
//! [`Dial`], the floor, and the points owed to the window — is the one struct
//! `Workspace`, and this module plus the dock block of `root_ui` are its only
//! readers. A pane is handed the state the workspace hangs off and is expected
//! to ignore all of it: the layout a pane is being drawn by is not a layout it
//! can be rearranging mid-pass.
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
//! before the UI runs in it — so a flag that lands the instant it is clicked
//! is drawn in a window that has not moved yet. Neither direction survives
//! that on its own: an unfold against the narrow window jumps every boundary
//! inward, and a fold against the wide one keeps the folded pane's whole
//! column, body gone and width held, which for the Spectral pane in a 1000pt
//! editor is a 207pt hole (the lattice, at 487, is a wider one). Deferring the
//! layout write does not reach the second — the
//! hole is what a collapsed flag draws at an un-narrowed fraction, and the
//! fraction is what is being deferred.
//!
//! So the FLAGS wait instead ([`Wait`]): the click is priced, the window is
//! asked, and the flags land on the frame that has it. The cost is a frame in
//! which nothing happens, which is a frame that looks exactly like the one
//! before it. A wait that cannot end is the thing to avoid, and neither way
//! out is optional — the window arrives, or it is not coming at all (a host
//! that refused, the floor it will not go under), and the second releases the
//! hold just as the first does.
//!
//! A whole subtree folds the same way, as many rails wide as it has panes that
//! end up beside each other: a collapsed column is one (its panes fold onto
//! each other's tab bars), a collapsed pair is two, and the split inside a
//! folded pair divides the width it was given into one rail each.
//!
//! The whole DOCK is the last of those, and it takes saying because nothing in
//! the tree says it. A split hands its freed width to its parent, so a fold is
//! always somebody's — except at the root, which has none, and where
//! [`folded_side`] therefore answers `None` however collapsed the tree is.
//! [`holds`] claims the root itself for that case, and [`paint`](fn@paint)
//! draws the strip from it: every pane a rail, the window down to what those
//! rails are worth, and each rail carrying the arrow that opens it. Left
//! unclaimed the one layout that is all rails would be the one layout read as
//! no fold at all — drawn at its full open width with nothing in it.
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
//! That difference is how a gesture ARRIVES; what it follows from there is the
//! pointer ([`Grip`]). A drag names a position, so every frame asks the same
//! question afresh — where is the pointer, and where was it when this boundary
//! was taken hold of — rather than adding up the steps in between. A sum has no
//! memory of what a clamp along the way refused, and there are clamps on both
//! sides of it: this module's own floor, and egui_dock's, which for a leaf is
//! the same one. So a boundary pushed 300 points past its floor would come away
//! 300 points from the hand still holding it, and set off again the instant the
//! pointer turned around. Following the pointer, the overshoot is owed and has
//! to be paid back before the pane grows, which is what every other resize
//! handle does.
//!
//! Only what is on screen can trade. A rail is a fixed number of points
//! whichever side of the boundary it sits, and a folded pane is holding the
//! width it comes back at rather than any width the drag can see — so the drag
//! reaches through them to the nearest pane that IS on screen, and leaves the
//! rest exactly where it is.
//! The separators drawn between those panes are on neither side of the boundary
//! either: hand them to one and every frame of a drag pushes the boundary
//! another separator into whichever subtree holds the more of them, so a
//! separator wiggled back and forth walks out from under the pointer
//! ([`Points::lean`]). What trades is the two panes the separator is DRAWN
//! between and only those, whatever the tree says their siblings are: shared out
//! in proportion instead, a drag on the settings edge moves the lattice too — 70
//! points of the 100 the settings column takes, in the plugin's own arrangement
//! — which is the whole picture re-laid-out for a gesture aimed at one edge of
//! it. At the floor of the pane it is drawn against the boundary stops
//! ([`Points::slack`]) rather than pushing on into the pane behind, which is the
//! same rule read backwards: it would leave a pane already as narrow as it goes
//! sliding sideways while a pane nobody pointed at paid for it.
//!
//! The separators a fold has PINNED cannot resize what they divide at all: the
//! one the rail sits against, and, where panes have folded side by side, the ones
//! between the rails. A rail is a fixed number of points, and the split that
//! folded it has its fraction rewritten every frame to keep it that way. But a
//! boundary with a pane somewhere to the left of it and a pane somewhere to the
//! right is a resize to everyone looking at it, whatever the tree says is
//! immediately either side, so that is what it does: the drag goes out to the
//! nearest split that divides two panes which can both change width
//! (`shove_target`,
//! `nudge`), and the rails travel with the boundary at their
//! own width. Every separator across a run of rails therefore moves the same two
//! panes, which is the only thing they could all mean.
//!
//! Where there is nothing outward to pass it to — a fold holding the whole of one
//! side of the window, one open pane and the window's own edge — there is no
//! second pane for the boundary to trade against, so the separators that fold
//! pinned have nothing to offer and are inert (`deaden`). What brings the pane
//! back there is the arrow on its rail, and only that.
//!
//! Whether a pane is folded stays egui_dock's own `collapsed` flag, set by its
//! own arrow: nothing here duplicates that bookkeeping, it only reads it.
//!
//! ## One tree, because a pane cannot leave the editor
//!
//! All of this is the MAIN surface's, and it is the only one there is: the dock
//! is built with `TabViewer::allowed_in_windows` off, so egui_dock's "Eject"
//! never offers a pane a window of its own. That is what lets the pass be one
//! tree instead of a loop over surfaces. A dock window is laid out in a window
//! whose size is not the editor's to know or to ask for, so every question this
//! module asks the window — what does the layout cost, has the ask been
//! answered, is this a refusal or a floor — has no answer out there, and each
//! one had to carry the case where the answer was "not applicable" alongside the
//! case where it meant something.
//!
//! A layout saved with a window in it still opens, and the panes in it are
//! dragged back the ordinary way.

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
    /// Which split, as an index into the tree. Valid only while the tree keeps
    /// its shape — the entry is dropped as soon as the node stops being folded,
    /// which covers re-docking.
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
    /// Zero in a blob written before it was recorded, which seeds nothing: the
    /// pane comes back at the rail width its fraction in the tree says, and one
    /// drag puts it where the user wants it.
    #[serde(default)]
    width: f32,
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

impl Folds {
    /// Bring the layout up to date with what the user did to it, and write the
    /// fractions that draw it in the window there is.
    ///
    /// Runs BEFORE the dock lays out, because a fraction is layout's input.
    ///
    /// The layout is [`Pass::points`]: a width per pane, held between frames and
    /// changed only by something the user did. Three things can have happened
    /// since the last pass, and each is a change to those widths:
    ///
    /// - a separator dragged, which moves points across a boundary ([`drags`]);
    /// - a pane folded or unfolded, which is the window's business rather than
    ///   the layout's — a folded pane KEEPS its points and is drawn as a rail
    ///   standing in for them, so unfolding restores nothing and cannot drift;
    /// - the window dragged, which shares itself out over the panes that are
    ///   visible ([`Points::refit`]) and leaves the folded ones alone. That is the whole
    ///   of "a resize does not scale a collapsed pane".
    ///
    /// What is DRAWN is then those points fitted to `area` ([`Points::spread`]): the
    /// rails at a fixed number of points, whatever is left shared out among the
    /// panes in proportion to what they are dialled at. When the window is the
    /// one the layout wants, that is the layout exactly; when it is not — a host
    /// that refused the fold's resize, or the floor it will not go under — the
    /// panes take the difference and the rails do not, which is the only place
    /// it can come from.
    ///
    /// Returns the points the window has to gain (negative: lose), which the
    /// shell asks its host for (see `Workspace::take_window_width_change`).
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
        let (gesturing, pointer) = (dial.gesturing, dial.pointer);
        let Some(tree) =
            dock.get_surface_mut(SurfaceIndex::main()).and_then(Surface::node_tree_mut)
        else {
            return 0.0;
        };
        if tree.is_empty() || !area.is_finite() || area <= 0.0 {
            return 0.0;
        }
        let holds = holds(tree);
        let pass = &mut dial.pass;
        let points = &mut pass.points;
        // A tree this pass has never seen: a fresh dock, a load, or one the
        // user has re-docked. The fractions in it are the only record of
        // what the panes are dialled to, so that is the seed — with the
        // folded ones put back to the width they were persisted at, since
        // their fraction in the tree is a rail's.
        if points.at.len() != tree.len() {
            points.seed(tree, area, separator);
            // The grip names a split by index, so a tree of another shape is
            // not the one the gesture took hold of.
            dial.grip = None;
            for fold in &self.0 {
                if fold.node >= tree.len() {
                    continue;
                }
                let node = NodeIndex(fold.node);
                if fold.width > 0.0 {
                    points.scale(tree, fold.side.of(node), fold.width);
                }
            }
        }
        // A separator the user dragged arrives as a fraction that has moved
        // off the one this pass wrote, and is followed from there by the
        // pointer — points moving across a boundary, which is all a drag
        // ever means.
        if gesturing {
            let (want, fixed) = wants(tree, &holds, &points.at, rail, separator);
            let gesture = Gesture { at: pointer, grip: &mut dial.grip };
            drags(tree, points, &holds, &want, &fixed, style, gesture);
        }
        // Whether the window is where the last pass left it. It answers two
        // questions that look unrelated and are the same one: a hold waiting
        // on a window that is not coming, and a layout whose ask the host
        // refused.
        let settled = (area - pass.area).abs() < 0.01;
        // An arrow's fold or unfold, held at the flags it had until the
        // window the layout is going to need arrives.
        let (holds, holding, lands) =
            hold_flags(tree, &mut dial.wait, &pass.flags, holds, area, settled);
        // The window a fold takes its width off is the one it was made in.
        // For a fold held across its own resize that is the window the hold
        // was armed at, not the narrower one it is landing in — the entry's
        // `window` is the ceiling a later unfold reads back off a persist
        // blob, and banking the post-fold width would lower it every time.
        //
        // A hold that OPENS a pane is the opposite case and must keep
        // `area`. Its landing frame makes entries too — a pair folded whole
        // holds no side, and opening one of its panes gives it one — but
        // the window that entry stands for is the wide one the pane has
        // just come back into, not the narrow one the unfold set off from.
        // Priced from `from`, every unfold would ratchet the ceiling down.
        let before =
            holding.as_ref().filter(|wait| lands && wait.shuts).map_or(area, |wait| wait.from);
        let moved = self.reconcile(tree, &holds, points, before);
        // The widest this window has been, for a session that was not there
        // to watch it get that wide: the folds came off the persist blob,
        // and each one remembers the window it was taken at.
        if dial.widest <= 0.0 {
            dial.widest =
                self.0.iter().fold(0.0_f32, |widest, fold| widest.max(fold.window));
        }
        let (want, fixed) = wants(tree, &holds, &points.at, rail, separator);
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
        //
        // A hold LANDING is the same refusal wearing `moved`, and has to be
        // read as one: its flags are moving on this pass, so `moved` is
        // true, but the ask they are moving on was made a pass ago and the
        // window has already declined it (see [`Wait`]).
        let refused = settled && dial.asked && (!moved || lands);
        let mut want = want;
        if (follow || refused) && area > floor + 1.0 && pass.area > 0.0 {
            points.refit(&want, &fixed, area);
            (want, _) = wants(tree, &holds, &points.at, rail, separator);
        }
        pass.area = area;
        // Only a fold or an unfold moves the window. Any other gap between
        // the layout and the window is one the window is not answering for
        // — a host that refused, or a floor it will not go under — and
        // asking again every frame would be an argument, not a request.
        //
        // Nothing is asked for on the frame a hold lands. The ask was made
        // when it was armed, and it priced the whole layout these flags
        // make — so asking again here would order the same resize twice
        // wherever the window has not answered the first.
        //
        // A pane HELD at the flags it had is priced at the layout it is
        // GOING to have rather than the one its flags still say — a pane
        // held shut waiting to open, or held open waiting to shut — and
        // that is the whole layout the window has to reach, a fold landing
        // on this same frame included. The holds it would leave behind come
        // off a copy: the shape is what `wants` reads the tree for, and
        // neither direction changes the shape, only the flags.
        let mut asking = 0.0;
        if !lands && (moved || holding.is_some()) {
            let want = match &holding {
                Some(wait) => {
                    let mut after = tree.clone();
                    restore(&mut after, &wait.landed);
                    wants(tree, &self::holds(&after), &points.at, rail, separator).0[0]
                }
                None => want[0],
            };
            // Never past the widest this window has actually been. Fold a
            // pane and drag the window back out, and every visible pane has
            // grown into the width the fold freed — so unfolding asks for
            // that width on top of a window that already spent it, which is
            // how a plugin window ends up wider than the display it is on.
            // Shrinking is never capped; only the growth is — and the ask
            // is not floored at zero either: fold one pane while another is
            // held and the net is a shrink, which flooring would swallow.
            dial.widest = dial.widest.max(area);
            asking = (want - area).min((dial.widest - area).max(0.0));
        }
        if let Some(wait) = holding.filter(|_| !lands) {
            // The width the hold waits for. An UNFOLD never waits for a
            // narrower window — where a fold landing on the same frame
            // makes the net ask a shrink, the pane it is holding still
            // opens into no less than what is already there, and waiting
            // on less would be waiting for a window that has come and
            // gone. A FOLD is the opposite: the narrower window is the
            // whole of what it is waiting for.
            //
            // A fold's target is held above the floor, which is the one
            // limit on the ask the shell applies and this pass can see
            // (`editor.rs` clamps to `MIN_SIZE`). Below it the ask names a
            // width the window will never report back, and the hold lands a
            // frame late through the "not coming at all" fallback instead
            // of on its own condition. The floor is a window width and this
            // is an area, so it is a bound rather than a boundary — which
            // is all it is asked to be.
            let at = if wait.shuts {
                (area + asking).max(floor)
            } else {
                area + asking.max(0.0)
            };
            dial.wait = Some(Wait { at: Some(at), ..wait });
        }
        // What this pass has actually put to the window, which is what the
        // next one reads a refusal against. A hold outstanding counts: it
        // has asked, and is waiting on the answer. A hold LANDING does not
        // — it asked a pass ago, and that ask has been answered by now
        // whichever way.
        dial.asked = asking.abs() > 0.01 || dial.wait.is_some();
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
        // Only where there IS a window to wait for. A resize the cap or the
        // floor has flattened to nothing leaves a frame of nothing followed
        // by a frame of nothing, and an unfold that waited (see [`Wait`])
        // lands on a frame whose window already fits it — wide enough for a
        // pane, narrow enough for a rail — which is exactly such a frame:
        // there is nothing to wait for and every reason to write the layout
        // now, because egui_dock re-derives its own fractions inside `show`
        // the moment a flag is off, and only a pass that has written first
        // can hold them.
        if !(moved && asking.abs() > 0.01) {
            points.spread(tree, &want, &fixed, area, separator);
        }
        // What this pass is leaving the flags at, which is what the next
        // one tells a fold or an unfold from. Taken at the END, so
        // everything the pass itself did — a hold put back, a hold let go
        // — is already in it and cannot read as somebody else's gesture.
        pass.flags = snapshot(tree);
        asking
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
        holds: &[Hold],
        points: &Points,
        window: f32,
    ) -> bool {
        let mut moved = false;
        // Unfolded, or re-docked out from under the entry.
        self.0.retain(|fold| {
            let held = holds.get(fold.node).and_then(|hold| hold.side) == Some(fold.side);
            moved |= !held;
            held
        });
        for (index, hold) in holds.iter().enumerate() {
            let Some(side) = hold.side else { continue };
            if self.0.iter().any(|fold| fold.node == index) {
                continue;
            }
            // First frame of this fold, priced against the window the pane
            // was last open in — which is what `window` is handed, whether
            // that is this frame's area or the one a hold was armed at a pass
            // ago (see [`Wait::from`]). It is the width this fold takes off
            // the window, and the ceiling a later unfold reads back.
            self.0.push(Fold {
                node: index,
                side,
                width: points.span(tree, &[], side.of(NodeIndex(index))),
                window,
            });
            moved = true;
        }
        moved
    }

    /// Forget every fold without handing anything back, for a load that brings
    /// its own window along with its layout. The entries name splits by index,
    /// so a tree they were not measured against has to start with none.
    ///
    /// [`Dial::forget`] goes with it wherever the dock itself is replaced.
    pub fn forget(&mut self) {
        self.0.clear();
    }

    /// Forget every fold, for a dock that is being replaced wholesale (the
    /// System pane's "Reset layout").
    ///
    /// Returns the points the window is owed for them. The layout that replaces
    /// this one has every pane open, so it wants the whole width the folds were
    /// keeping off the window.
    ///
    /// Priced off `dock` here rather than banked by [`Folds::apply`] every
    /// frame, because this is the only thing that ever reads it: a button
    /// pressed about as often as never does not need a tree walk per frame
    /// standing ready for it. It has to be the dock being THROWN AWAY, folds
    /// and dialled widths and all — the one replacing it has neither.
    #[must_use]
    pub fn clear(
        &mut self,
        dock: &DockState<Tab>,
        style: &egui_dock::Style,
        dial: &Dial,
        area: f32,
    ) -> f32 {
        self.forget();
        // What the layout would want with every pane OPEN, which is what the
        // reset hands back.
        let open = dock
            .get_surface(SurfaceIndex::main())
            .and_then(Surface::node_tree)
            .filter(|tree| !tree.is_empty())
            .map_or(0.0, |tree| {
                wants(tree, &[], &dial.pass.points.at, style.tab_bar.height, style.separator.width)
                    .0[0]
            });
        // Held to the same ceiling as the rail's arrow (see the ask in
        // [`Folds::apply`]), because it undoes the same fold and therefore owes
        // the same width. A layout wanting a window that never arrived — a host
        // that refused, or a drag back out while folded — prices itself well
        // above anything the window has been, and a button that hands that
        // price to the host is how the editor ends up wider than the display.
        let want = if open > area { open - area } else { 0.0 };
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

/// What the shell carries between frames: the layout in points, and what the
/// window is doing to it.
///
/// Runtime-only, and seeded from the fractions it finds in a tree it has no
/// points for — a re-dock, or a dock handed over by [`Dial::forget`]. A tree of
/// unchanged shape is NOT that: folding leaves the node count alone, so a
/// layout arriving over one already dialled has to say so rather than be
/// noticed.
#[derive(Default)]
pub struct Dial {
    /// What the last pass left the layout at.
    pass: Pass,
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
    /// Where the pointer is, along the axis a horizontal separator moves in.
    /// What a gesture on a boundary asks for is a POSITION, not a sum of the
    /// steps it took to get there (see [`Grip`]).
    pointer: Option<f32>,
    /// The boundary this gesture has hold of, if it has hold of one.
    grip: Option<Grip>,
    /// A fold or an unfold waiting for the window its layout needs.
    wait: Option<Wait>,
}

/// What one pass leaves the next.
#[derive(Default)]
struct Pass {
    /// The layout, in points.
    points: Points,
    /// The area it was laid out in, so a window that moved on its own can be
    /// told from one that is answering an ask of this pass's.
    area: f32,
    /// The collapsed flags it was left with, so the next pass can tell a fold or
    /// an unfold that has happened since from the state it was already in.
    flags: Vec<Flags>,
}

/// A pane that has just been folded or unfolded, held at the flags it had
/// until the window the new layout needs arrives.
///
/// The wait is the whole point. egui_dock re-derives its own fractions from
/// inside `DockArea::show` the moment a collapsed flag moves, and the resize
/// that flag asks for cannot land until the frame after — so flags that land
/// against the old window get laid out in the window they are LEAVING, for
/// that frame.
///
/// Both directions need it, for artifacts that do not look alike. An unfold
/// against a too-narrow window jumps every boundary on screen inward. A fold
/// against a too-wide one moves no boundary at all — the folded pane simply
/// KEEPS its whole column, body gone and width held, so a pane worth a fifth
/// of the editor reads as a wide empty hole until the window closes over it.
/// Deferring the layout write does not reach that one: the hole is what a
/// collapsed flag draws at an un-narrowed fraction, and the fraction is the
/// thing being deferred.
///
/// Held by watching the FLAGS rather than by intercepting a click, because the
/// click is not ours to intercept: egui_dock draws its collapse button for a
/// folded leaf too, and expanding there is its own `set_collapsed` from inside
/// `show`. Both that button and the rail's own arrow reach here the same way.
#[derive(Clone)]
struct Wait {
    /// The flags as the gesture left them, to put back on the frame that has
    /// the window. Kept whole rather than as the pane that moved: what
    /// egui_dock sets reaches up the ancestors as well, and a snapshot needs to
    /// know none of that.
    landed: Vec<Flags>,
    /// The window this gesture has asked for, once it has asked — the ask is
    /// made on the frame the gesture is caught and answered on the next, so the
    /// first frame of a `Wait` has nothing here yet.
    ///
    /// What was ASKED for rather than what the layout wants, because the two
    /// differ wherever the growth cap bites: waiting on a width the ask never
    /// requested is waiting forever.
    at: Option<f32>,
    /// The window the hold was armed at, which is the one the fold it may be
    /// carrying takes its width off (see [`Folds::reconcile`]).
    from: f32,
    /// Whether the held flags CLOSE a pane, which is what says the window has
    /// to get narrower before they may land — and so which way [`Wait::at`] is
    /// waited for.
    shuts: bool,
}

/// Catch the fold or unfold the user has just made and hold its flags back
/// until the window its layout needs is there ([`Wait`]), then let them land on
/// the frame that has it.
///
/// Answers the holds the rest of the pass works from — re-read wherever the
/// flags moved either way — the hold outstanding, which is what prices the
/// window the layout is heading for, and whether that hold has just landed.
///
/// Caught by comparing `was`, the flags the last pass left, against the ones in
/// the tree, rather than by intercepting the click that moved them: the click is
/// not always ours to intercept (see [`Wait`]).
fn hold_flags(
    tree: &mut Tree<Tab>,
    wait: &mut Option<Wait>,
    was: &[Flags],
    holds: Vec<Hold>,
    area: f32,
    settled: bool,
) -> (Vec<Hold>, Option<Wait>, bool) {
    let moving = wait.is_none()
        && was.len() == tree.len()
        && {
            // What the window is asked for is what the HOLDS say, not what the
            // flags say. A pane folded downwards moves a flag and moves no hold
            // — `folded_side` reads horizontal splits and nothing else, so
            // egui_dock does the whole of that one and there is no width to take
            // off the window. Waiting on a resize nobody is going to ask for
            // costs the click a frame and buys nothing, and the settings
            // column's Notes/Console bar is the arrow it would be bought on.
            let mut had = tree.clone();
            restore(&mut had, was);
            self::holds(&had).iter().map(|hold| hold.side).ne(holds.iter().map(|hold| hold.side))
        };
    if moving {
        // The tree goes back to where the last pass left it, and what the
        // gesture did is kept for the frame that can afford it.
        let landed = snapshot(tree);
        // Which way the window has to go before the flags may land: a flag going
        // ON is a fold, and the layout it wants is narrower than the one on
        // screen.
        let shuts = (0..tree.len()).any(|node| !was[node].0 && landed[node].0);
        restore(tree, was);
        *wait = Some(Wait { landed, at: None, from: area, shuts });
    }
    let holding = wait.clone();
    // The window is here, or it is never coming: either way there is nothing
    // left to wait for, and the rest of the pass is the fold itself, laid out in
    // the window it was priced for.
    //
    // Within half a point counts as arrived, because that is as close as the
    // window can be put: the shell asks in whole points (`editor.rs` rounds),
    // and a layout wanting a fractional window is where every fold lands. Held
    // to the exact width, a hold would wait on a window that has already come as
    // near as it can go.
    let lands = holding.as_ref().is_some_and(|held| {
        held.at.is_some_and(|at| {
            let arrived = if held.shuts { area <= at + 0.5 } else { area >= at - 0.5 };
            arrived || settled
        })
    });
    if lands {
        if let Some(held) = &holding {
            restore(tree, &held.landed);
        }
        *wait = None;
    }
    // Folds are re-read after a held gesture either way, which has just moved
    // flags.
    let holds = if moving || lands { self::holds(tree) } else { holds };
    (holds, holding, lands)
}

/// One node's share of what a fold is, as the tree holds it: whether it is
/// collapsed, and how many collapsed leaves are under it (which is what makes
/// a rail as many tab bars wide as it has panes side by side).
///
/// The count is a SPLIT's, and asking a leaf for it panics inside egui_dock, so
/// it is carried as zero for everything that is not a parent and put back only
/// where it came from.
type Flags = (bool, i32);

fn snapshot(tree: &Tree<Tab>) -> Vec<Flags> {
    (0..tree.len())
        .map(|index| {
            let node = &tree[NodeIndex(index)];
            if node.is_empty() {
                return (false, 0);
            }
            (node.is_collapsed(), if node.is_parent() { node.collapsed_leaf_count() } else { 0 })
        })
        .collect()
}

/// Put a snapshot back, where the tree still has the shape it was taken from.
/// A re-dock changes that shape, and the snapshot is dropped rather than
/// applied — a fold held against a tree it was not measured on is worse than
/// no hold at all.
fn restore(tree: &mut Tree<Tab>, flags: &[Flags]) {
    if flags.len() != tree.len() {
        return;
    }
    for (index, &(collapsed, leaves)) in flags.iter().enumerate() {
        let node = &mut tree[NodeIndex(index)];
        if node.is_empty() {
            continue;
        }
        node.set_collapsed(collapsed);
        if node.is_parent() {
            node.set_collapsed_leaf_count(leaves);
        }
    }
}

impl Dial {
    /// Drop everything that names the tree by index, for a dock being replaced
    /// wholesale — a reset, or a load that brings its own layout.
    ///
    /// Folding never changes the tree's SHAPE, so a fresh default dock has the
    /// same node count as the one it replaces and neither length guard here can
    /// tell them apart. Both of the things keyed by that count have to be said
    /// out loud, then:
    ///
    /// - the FLAGS, or the stale ones read as an unfold the user did, and the
    ///   pane the reset just opened is put straight back for a frame;
    /// - the POINTS, or the widths survive the layout that was dialled to them.
    ///   The fractions in the incoming dock are the only record of what the new
    ///   layout wants, and [`Folds::apply`] reads them only when it finds a tree
    ///   it has no points for — so a reset that keeps its points resets the
    ///   arrangement around widths the user dragged, and nothing moves.
    ///
    /// The grip goes with them: it names a split by index in a layout that is
    /// being thrown away.
    ///
    /// What does NOT go is [`Dial::widest`], which is the widest this window has
    /// been rather than anything about this layout. An unfold may not ask the
    /// host for a window wider than one it has already granted, and a reset that
    /// forgot that ceiling would have to learn it again from a resize.
    pub fn forget(&mut self) {
        self.pass = Pass::default();
        self.grip = None;
        self.wait = None;
    }
}

/// What the pointer is doing to the layout this frame.
struct Gesture<'a> {
    /// Where the pointer is, along the axis a horizontal separator moves in.
    at: Option<f32>,
    grip: &'a mut Option<Grip>,
}

/// A boundary a gesture has hold of: where it stood when the gesture reached it,
/// and where the pointer was then.
///
/// A drag names a position. Once a boundary is held, what it follows is the
/// pointer's own travel from the grip — never the sum of the steps in between,
/// which is what every clamp along the way eats a piece of. Dragged past a floor,
/// the pane stops and the points the drag could not spend are still owed: the
/// pointer has to come back past the floor before the pane grows again, rather
/// than the handle setting off the moment the pointer turns around, an overshoot
/// away from the hand still holding it.
///
/// It is also the only way to see a drag egui_dock has stopped answering for. Its
/// own clamp holds `separator.extra` points of pane either side of the separator
/// being dragged, which for a leaf is the same floor this holds — so past it the
/// fraction saturates, and a boundary read off the fraction would have nothing
/// left to read.
#[derive(Clone, Copy)]
struct Grip {
    node: usize,
    /// The points of pane before the boundary when it was taken hold of.
    at: f32,
    /// Where the pointer was then.
    from: f32,
    /// The points of pane before it as this pass last left them. A boundary that
    /// has moved since is one something OTHER than this gesture moved — a fold,
    /// an unfold, a window that would not shrink — and a grip taken in the layout
    /// before that is not holding anything in this one.
    left: f32,
}

/// The layout, in points.
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
    /// The boundary a gesture has hold of, for the chrome that has to show it
    /// (see [`paint`](fn@paint)).
    pub(crate) fn held(&self) -> Option<NodeIndex> {
        self.grip.map(|grip| NodeIndex(grip.node))
    }

    /// What the shell saw of the pointer this frame, before [`Folds::apply`]
    /// reads the fractions the last one left behind: whether it is doing
    /// anything, and where it is.
    pub fn watch_pointer(&mut self, gesturing: bool, at: Option<f32>) {
        self.gesturing = gesturing;
        self.pointer = at;
        // Nothing is being held once the pointer is up, and the next gesture
        // takes hold of its own boundary wherever that one stands.
        if !gesturing {
            self.grip = None;
        }
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
    /// wide — how a width names a layout, whether it came off a drag or a
    /// persist blob.
    fn scale(&mut self, tree: &Tree<Tab>, node: NodeIndex, width: f32) {
        let was = self.span(tree, &[], node);
        if was <= 0.0 || !width.is_finite() || width <= 0.0 {
            return;
        }
        self.stretch(tree, node, width / was);
    }

    /// The points a subtree is dialled to, side by side: a vertical split's two
    /// children are the same width rather than two widths, so this is not the
    /// sum of the leaves under it.
    ///
    /// Counts only what `holds` leaves on SCREEN, which is what a drag can
    /// trade — pass no holds for the whole of what a subtree is dialled to,
    /// folded panes included, which is what a fold is worth to the window.
    fn span(&self, tree: &Tree<Tab>, holds: &[Hold], node: NodeIndex) -> f32 {
        if holds.get(node.0).is_some_and(|hold| hold.inside) {
            return 0.0;
        }
        let (left, right) = (node.left(), node.right());
        match &tree[node] {
            Node::Leaf(_) => self.at.get(node.0).copied().unwrap_or(0.0),
            _ if right.0 >= tree.len() => 0.0,
            Node::Vertical(_) => {
                self.span(tree, holds, left).max(self.span(tree, holds, right))
            }
            Node::Horizontal(_) => self.span(tree, holds, left) + self.span(tree, holds, right),
            Node::Empty => 0.0,
        }
    }

    /// Move `by` points into the pane under `node` that FACES the boundary — the
    /// half of a drag that lands on one side of it. `edge` is the side of `node`
    /// the boundary sits on, so `Side::Right` for the subtree to the LEFT of it.
    ///
    /// What a separator resizes is the two panes it is drawn between, whatever
    /// the tree says those panes' siblings are. Shared out over the subtree in
    /// proportion to what each pane is dialled at, every pane on that side moves
    /// at once: in the plugin's own arrangement, dragging the settings edge 100
    /// points takes 70 of them off the LATTICE and 30 off the analyzer, so a
    /// gesture aimed at one edge of the picture re-lays-out the whole of it.
    ///
    /// At that pane's floor the boundary STOPS, rather than pushing on into the
    /// pane behind it. Pushing on is what a row of sashes does elsewhere, and it
    /// is the same rule read the other way round: the settings edge would go on
    /// travelling by taking width off the LATTICE, with the analyzer sliding
    /// sideways at a fixed width between them — a handle nobody grabbed moving,
    /// which is what a drag that has run out of room looks like from the outside
    /// anyway. Stopping owes the pointer nothing it cannot pay: the boundary
    /// follows the pointer's travel from where it took hold ([`Grip`]), so an
    /// overshoot is repaid on the way back rather than lost.
    ///
    /// What it does pass OVER is a folded subtree, which is not on screen for
    /// the separator to be drawn against at all — the rails between a boundary
    /// and the nearest open pane are chrome, and the drag reaches through them.
    ///
    /// Named as a difference rather than as a width, because the width a subtree
    /// COMES OUT at is not the width its panes hold between them: the separators
    /// drawn between them are part of the first and none of the second, and a
    /// drag hands them to neither side. Handed the wrong one of the two, every
    /// frame of a drag pushes the boundary a separator further into whichever
    /// subtree has the more of them, and a separator wiggled back and forth walks
    /// out from under the pointer.
    ///
    /// `floors` is per node and in the same points as `by`: what the panes under
    /// it need between them, rails and separators left out. Returns what the
    /// floor refused, which for a drag is nothing — [`drags`] clamps the delta to
    /// [`Points::slack`], the same pane's room, before it asks for any of it.
    fn lean(
        &mut self,
        tree: &Tree<Tab>,
        holds: &[Hold],
        floors: &[f32],
        node: NodeIndex,
        by: f32,
        edge: Side,
    ) -> f32 {
        // A rail is a fixed number of points and a folded pane is holding the
        // width it comes back at: neither is on screen for a drag to move, so
        // what the boundary is asking for passes over them to the next pane in.
        if holds.get(node.0).is_some_and(|hold| hold.inside) || !by.is_finite() {
            return by;
        }
        let (left, right) = (node.left(), node.right());
        match &tree[node] {
            Node::Leaf(_) => {
                let floor = floors.get(node.0).copied().unwrap_or(0.0);
                let Some(at) = self.at.get_mut(node.0) else { return by };
                // Its floor bounds how far it can be SQUEEZED and nothing else:
                // lifting a pane already under its floor would spend more than
                // the drag asked for, which the other side of the boundary is
                // not giving up and the window has nowhere to take from.
                let take = by.max((floor - *at).min(0.0));
                *at += take;
                by - take
            }
            _ if right.0 >= tree.len() => by,
            Node::Empty => by,
            // Stacked, so both children are the same width and both of them
            // reach the boundary: the column moves as one, and no further than
            // the shallower of the two can go. Which is [`Points::slack`] and
            // NOT this node's own floor: where a row is a side-by-side pair,
            // what faces the boundary is half a row, and it runs out of room
            // long before the row it is in does.
            Node::Vertical(_) => {
                let take = by.max(-self.slack(tree, holds, floors, node, edge));
                self.lean(tree, holds, floors, left, take, edge);
                self.lean(tree, holds, floors, right, take, edge);
                by - take
            }
            _ => self.lean(tree, holds, floors, self.facing(tree, holds, node, edge), by, edge),
        }
    }

    /// How far the pane a separator is drawn against can be squeezed: what it
    /// holds over its own floor, and nothing the panes behind it hold.
    ///
    /// [`drags`] clamps the drag to this, so [`Points::lean`] is always asked
    /// for something it can spend in full — a boundary that stopped where the
    /// floor is while the gesture went on believing it had moved would leave the
    /// two sides of it trading different numbers of points, and the layout no
    /// longer summing to the window it is drawn in.
    ///
    /// A COLUMN is worth the least of its rows, because all of them are drawn at
    /// one width and all of them face the boundary: the row that runs out first
    /// is where the column stops. Not the column's own floor, which is a
    /// different number — `floors` is `min_widths` less what a fold has fixed,
    /// and a vertical split takes the greater of its rows for each of those two
    /// separately, so where the greater comes from different rows the difference
    /// is a width no pane in the column is holding.
    fn slack(
        &self,
        tree: &Tree<Tab>,
        holds: &[Hold],
        floors: &[f32],
        node: NodeIndex,
        edge: Side,
    ) -> f32 {
        if holds.get(node.0).is_some_and(|hold| hold.inside) {
            return 0.0;
        }
        let (left, right) = (node.left(), node.right());
        match &tree[node] {
            Node::Leaf(_) => {
                let floor = floors.get(node.0).copied().unwrap_or(0.0);
                (self.at.get(node.0).copied().unwrap_or(0.0) - floor).max(0.0)
            }
            _ if right.0 >= tree.len() => 0.0,
            Node::Empty => 0.0,
            Node::Vertical(_) => self
                .slack(tree, holds, floors, left, edge)
                .min(self.slack(tree, holds, floors, right, edge)),
            _ => self.slack(tree, holds, floors, self.facing(tree, holds, node, edge), edge),
        }
    }

    /// Which child of a horizontal split the separator on `edge` is drawn
    /// against: the near one, unless it is folded away to rails and has no pane
    /// on screen to be drawn against at all.
    fn facing(&self, tree: &Tree<Tab>, holds: &[Hold], node: NodeIndex, edge: Side) -> NodeIndex {
        let (near, far) = match edge {
            Side::Left => (node.left(), node.right()),
            Side::Right => (node.right(), node.left()),
        };
        match self.span(tree, holds, near) > 0.0 {
            true => near,
            false => far,
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
                let asked = sl + sr;
                // A window within a point of the layout IS the layout. Scaling
                // that last fraction out over both sides moves every boundary
                // by a share of it, which reads as the whole arrangement
                // sliding; handing the left child exactly what it asked for
                // walks the difference rightward instead, split by split, until
                // the last pane wears it whole. A rounding's worth on one edge
                // is invisible where the same amount on all of them is not, and
                // the leftmost pane is the one the eye is anchored to.
                //
                // Not an edge case: the shell can only ask for whole points
                // (`editor.rs` rounds the ask, and drops one under half a
                // point), so a layout wanting a fractional window is where
                // every fold lands.
                let before = if asked > 0.0 && (slack - asked).abs() < 1.0 {
                    fl + sl
                } else {
                    let by = if asked > 0.0 { (slack / asked).max(0.0) } else { 0.0 };
                    fl + sl * by
                };
                let before = before.clamp(0.0, (width - separator).max(0.0));
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
    // nothing fixed about it at all — what has piled up in `fixed` there is one
    // separator per horizontal split and no rail at all, which is a width the
    // panes wear like any other rather than one held back from them.
    //
    // A dock folded WHOLE has no side to read: the root is claimed as `inside`
    // and nothing under it names a folded child. It is still every bit a fold,
    // and its rails are still fixed, which is what `folded` says and `side`
    // alone does not.
    if !holds.iter().any(Hold::folded) {
        fixed = vec![0.0; tree.len()];
    }
    (want, fixed)
}

/// Read the separator the user is dragging back into the layout: points moving
/// across a boundary, which is all a drag ever means.
///
/// The gesture ARRIVES as a fraction. It lands on the one this pass wrote, one
/// frame after the rectangles it was measured against were drawn, so the
/// difference from what was written times the width that split came out at is
/// the points the boundary has travelled. No inverse is needed for it, and no
/// bisection: the layout and the thing the user pointed at are in the same units.
///
/// It is FOLLOWED by the pointer. Once the boundary is held ([`Grip`]) the
/// fraction is not read again — what is asked for is where the pointer is, every
/// frame from the position it was taken hold of, so that neither a floor nor
/// egui_dock's own clamp can quietly eat a piece of the gesture on the way past.
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
    gesture: Gesture,
) {
    let Gesture { at: pointer, grip } = gesture;
    let separator = style.separator.width;
    let floor = style.separator.extra - separator * 0.5;
    let mins = min_widths(tree, floor, style.tab_bar.height, separator);
    // Per node, what its panes need between them: the rails and the separators
    // are in both of these and cancel, which leaves the same points a drag is in.
    let floors: Vec<f32> =
        mins.iter().zip(fixed).map(|(min, fixed)| (min - fixed).max(0.0)).collect();
    let drawn = points.drew.clone();
    // The drag is in DRAWN points, and the layout is in dialled ones. They are
    // the same thing at the window the layout wants, and a fixed ratio apart at
    // the one a host refused it (see [`Points::spread`]).
    let (dialled, whole) = (want[0] - fixed[0], drawn[0] - fixed[0]);
    if dialled <= 0.0 || whole <= 0.0 {
        return;
    }
    for index in 0..tree.len() {
        let node = NodeIndex(index);
        let (left, right) = (node.left(), node.right());
        if right.0 >= tree.len() || !tree[node].is_horizontal() {
            continue;
        }
        // One pointer, one boundary: once this gesture has hold of one, no other
        // separator's fraction is anything but egui_dock re-clamping it.
        let mut held = *grip;
        if held.is_some_and(|grip| grip.node != index) {
            continue;
        }
        // Only what is on SCREEN can trade: a rail is a fixed number of points
        // whichever side of the boundary it is on, and a folded pane is holding
        // the width it comes back at rather than any width the drag can see.
        let (before, after) = (want[left.0] - fixed[left.0], want[right.0] - fixed[right.0]);
        if before <= 0.0 || after <= 0.0 {
            continue;
        }
        let standing = points.span(tree, holds, left);
        // Let go of a boundary something else has moved: the gesture is still on
        // screen where it was, but what it is over is a different layout.
        if held.is_some_and(|grip| (standing - grip.left).abs() > 0.5) {
            (*grip, held) = (None, None);
        }
        let asked = match held {
            // Held: where the pointer is, which is the whole of what a drag ever
            // meant. Every frame answers the same question afresh, so nothing a
            // clamp refuses along the way is lost or double-counted.
            Some(grip) => {
                let Some(at) = pointer else { continue };
                grip.at + (at - grip.from) * dialled / whole
            }
            // Not yet: a fraction that has moved off the one this pass wrote is
            // the gesture arriving, and this is the frame it is taken hold of.
            None => {
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
                // The boundary has already travelled this frame's worth, so the
                // pointer was that far back when it stood where the layout still
                // says it does.
                let travelled = moved * drew;
                if let Some(at) = pointer {
                    let (at, from) = (standing, at - travelled);
                    *grip = Some(Grip { node: index, at, from, left: standing });
                }
                standing + travelled * dialled / whole
            }
        };
        // Held to what the two panes it is DRAWN between can give up, which is
        // neither what egui_dock's own limit holds — that one holds the split's
        // two CHILDREN apart and nothing deeper in either of them (see
        // [`min_widths`]) — nor what those two subtrees hold between them.
        let (spare, take) = (
            points.slack(tree, holds, &floors, left, Side::Right),
            points.slack(tree, holds, &floors, right, Side::Left),
        );
        let delta = (asked - standing).clamp(-spare, take);
        if delta.abs() < 1e-4 {
            continue;
        }
        // Each side's own edge of the boundary, so the panes the separator is
        // drawn between are the ones that move. Both sides spend the drag in
        // full, since it was priced at what each of them had: a side that
        // silently spent less is one trading fewer points than the other across
        // the same boundary, and a layout that stops summing to its window.
        let unspent = points.lean(tree, holds, &floors, left, delta, Side::Right).abs()
            + points.lean(tree, holds, &floors, right, -delta, Side::Left).abs();
        debug_assert!(unspent < 1e-3, "{unspent}pt of a {delta}pt drag went unspent");
        if let Some(grip) = grip {
            grip.left = points.span(tree, holds, left);
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
    // A fold at the ROOT is nobody's to hand over. `folded_side` answers `None`
    // for a split with both children collapsed, because the width such a split
    // gives up is its PARENT's to take — and the root has no parent, so the one
    // tree folded WHOLE is the one tree nothing else reads as folded at all.
    // Claiming it here is what makes an all-folded dock a strip of rails rather
    // than a full-width window with nothing in it.
    if !tree.is_empty() && tree[NodeIndex::root()].is_collapsed() {
        holds[NodeIndex::root().0].inside = true;
    }
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
            holds[side.of(node).0].inside = true;
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
