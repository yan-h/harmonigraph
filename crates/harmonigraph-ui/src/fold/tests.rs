//! Unit tests for the fold layout.

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

/// The dock chrome as `theme::dock_style` sets it, in the numbers that
/// matter here: a tab bar's height, a separator's width, and the floor a pane
/// can be dragged to.
///
/// The floor especially. egui_dock's own default is 175pt, which the app
/// replaces (`theme::min_pane`) — and leaving the default here means every
/// test measures drags against a limit the editor does not have, in a module
/// whose whole subject is which fraction ends up in which split.
fn style() -> egui_dock::Style {
    let mut style = egui_dock::Style::from_egui(&egui::Style::default());
    style.tab_bar.height = 26.0;
    style.separator.width = 4.0;
    style.separator.extra = 4.0 * 26.0;
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
