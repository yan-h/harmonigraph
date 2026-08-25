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
    let [pictures, settings] = surface.split_right(NodeIndex::root(), 0.7, vec![Tab::Tuning]);
    surface.split_below(settings, 0.5, vec![Tab::Notes]);
    surface.split_right(pictures, 0.7, vec![Tab::Spectral]);
    dock
}

/// The dock chrome the fold layout is run with, off the app's own
/// [`theme::dock_style`](crate::theme::dock_style) rather than restated here.
///
/// The floor especially. egui_dock's own default is 175pt, which the app
/// replaces (`theme::min_pane`) — and leaving the default here means every
/// test measures drags against a limit the editor does not have, in a module
/// whose whole subject is which fraction ends up in which split. Three numbers
/// out of that style reach the fold at all — the tab bar's height, the
/// separator's width, and that floor — and taking them from the source is what
/// keeps a retune of any of the three from leaving this file measuring a dock
/// the app does not have.
///
/// At the design scale, because the fold layout is scale-free: it is handed
/// widths in points and the style tells it what a rail costs, so a sweep over
/// chrome scales here would only re-run the same arithmetic on bigger numbers.
fn style() -> egui_dock::Style {
    crate::theme::dock_style(&egui::Style::default(), 1.0)
}

/// The width a folded pane leaves behind — one tab bar — off that same style.
fn rail() -> f32 {
    style().tab_bar.height
}

/// What the dock does between two [`Folds::apply`] calls: hand every
/// split's rectangle down to its children the way `compute_rect_sizes`
/// does, so a test can watch a fold settle the way a frame would.
///
/// Vertical splits are laid out by their fraction alone, without
/// egui_dock's collapsed-leaf rule. The fold reads the WIDTH of horizontal
/// splits and nothing else, so the difference never reaches it.
fn lay_out(dock: &mut DockState<Tab>, width: f32) {
    let separator = style().separator.width;
    let frame = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, FRAME_HEIGHT));
    let tree = &mut dock[SurfaceIndex::main()];
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
/// `Workspace::take_window_width_change`). Tests thread it the same way.
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
        let leaves = if tree[parent].is_horizontal() { below.max(beside) } else { below + beside };
        tree[parent].set_collapsed_leaf_count(leaves);
        child = parent;
    }
}

/// Node indices, named as the plugin's dock lays them out.
const PICTURES: NodeIndex = NodeIndex(1);
const SETTINGS: NodeIndex = NodeIndex(2);
const LATTICE: NodeIndex = NodeIndex(3);
const SPECTRAL: NodeIndex = NodeIndex(4);

/// [`dock`]'s claim about node indices, checked against the layout it stands
/// in for rather than trusted.
///
/// The FRACTIONS and the tab set are the fixture's own and deliberately not
/// the plugin's: these tests dial widths themselves and read them back, and
/// the real default opens with a leaf already collapsed, which is a state
/// half of them are about reaching. What has to match is the SHAPE — the same
/// splits in the same order — because that is what makes [`PICTURES`],
/// [`SETTINGS`], [`LATTICE`] and [`SPECTRAL`] name the nodes they claim to.
/// A split reordered in `state::default_dock` renumbers every one of them, and
/// nothing else here would notice: the tests would go on passing about
/// whichever panes those indices landed on.
#[test]
fn the_fixture_dock_has_the_shape_of_the_plugins_own() {
    let fixture = dock();
    let real = crate::state::default_dock();
    let kind = |dock: &DockState<Tab>, node: NodeIndex| match &dock[SurfaceIndex::main()][node] {
        Node::Horizontal(_) => "horizontal",
        Node::Vertical(_) => "vertical",
        Node::Leaf(_) => "leaf",
        Node::Empty => "empty",
    };
    let tree = &real[SurfaceIndex::main()];
    assert_eq!(
        fixture[SurfaceIndex::main()].len(),
        tree.len(),
        "the fixture dock has a different number of nodes than the plugin's",
    );
    for index in 0..tree.len() {
        let node = NodeIndex(index);
        assert_eq!(
            kind(&fixture, node),
            kind(&real, node),
            "node {index} is a different kind of node in the two docks",
        );
    }
    // ...and the four indices these tests name really are those panes, in BOTH
    // docks. The kinds above cannot tell one leaf from another, so without this
    // the fixture could hold the lattice and the analyzer the other way round
    // and still match.
    let under = |root: NodeIndex, of: NodeIndex| {
        let mut walk = Some(of);
        while let Some(n) = walk {
            if n == root {
                return true;
            }
            walk = n.parent();
        }
        false
    };
    for (name, node, tab) in [
        ("PICTURES", PICTURES, Tab::Lattice),
        ("SETTINGS", SETTINGS, Tab::Tuning),
        ("LATTICE", LATTICE, Tab::Lattice),
        ("SPECTRAL", SPECTRAL, Tab::Spectral),
    ] {
        for (whose, dock) in [("the plugin's own", &real), ("the fixture", &fixture)] {
            let found = dock.find_tab(&tab).expect("both docks hold every tab named here");
            assert!(under(node, found.node), "{name} does not contain {tab:?} in {whose} dock");
        }
    }
}

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
        (window - (1000.0 - (pane - rail()))).abs() < 0.01,
        "the window loses the pane's width, less the rail standing in for it"
    );
    // The frame after, at the size the window was asked for, is where the
    // layout settles — and asks for nothing more.
    let settled = frame(&mut folds, &mut dock, &mut dial, window);
    assert!((settled - window).abs() < 0.01, "one fold, one resize");
    assert!((width(&dock, LATTICE) - rail()).abs() < 0.01, "a rail, not a column");
    assert!((width(&dock, SPECTRAL) - sibling).abs() < 0.01, "the analyzer keeps the width it had");
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
            (width(&dock, LATTICE) - rail()).abs() < 0.01,
            "at {size} wide, in the {window} the fold asked for"
        );
    }
}

/// Unfolding is the layout coming back, not a guess at a new one — and it
/// comes back on the frame the WINDOW does. The click alone changes nothing
/// on screen: the frame it lands on is still being drawn in the window the
/// fold took its width out of, and drawing the unfolded layout there would
/// squeeze every pane to fit a window that is one frame from growing.
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
    assert!((window - 1000.0).abs() < 0.01, "the window it came out of");
    let _ = frame(&mut folds, &mut dock, &mut dial, window);
    assert!((fraction(&dock, PICTURES) - 0.7).abs() < 0.001, "and the layout with it");
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

/// A window dragged while a pane is folded is worn by the panes that are on
/// SCREEN to wear it. The folded one is not being drawn at all, so it has no
/// share of that window to take, and it comes back exactly the width it went
/// away — which is what makes a fold reversible across a resize rather than
/// merely repeatable.
///
/// The layout this lands in is therefore NOT the one the resize alone would
/// have reached, and that is the point. A folded pane given its share of a
/// window it spent folded hands that share back on the way out, so unfolding
/// re-lays-out panes nobody touched: "opening a pane made the other two
/// smaller" is what that looks like from the outside.
#[test]
fn a_resize_while_a_pane_is_folded_is_worn_by_the_panes_on_screen() {
    let mut dock = dock();
    let mut folds = Folds::default();
    let mut dial = Dial::default();
    // A settled frame before the click, as the editor always has: the
    // layout is drawn in the window it is dialled for.
    let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
    let pane = width(&dock, LATTICE);
    collapse(&mut dock, Tab::Lattice, true);
    let window = settle(&mut folds, &mut dock, &mut dial, 1000.0);
    let open = [SPECTRAL, SETTINGS].map(|node| width(&dock, node));

    // The user drags the window border in while the lattice is a rail.
    let dragged = window - 120.0;
    let _ = settle(&mut folds, &mut dock, &mut dial, dragged);
    let now = [SPECTRAL, SETTINGS].map(|node| width(&dock, node));
    let lost: f32 = open.iter().zip(now).map(|(open, now)| open - now).sum();
    assert!(
        (lost - 120.0).abs() < 0.5,
        "the two panes on screen should have worn the whole 120pt, and wore {lost}",
    );
    assert!(
        (width(&dock, LATTICE) - rail()).abs() < 0.01,
        "while the rail stayed a rail: {}",
        width(&dock, LATTICE),
    );

    collapse(&mut dock, Tab::Lattice, false);
    let _ = settle(&mut folds, &mut dock, &mut dial, dragged);
    assert!(
        (width(&dock, LATTICE) - pane).abs() < 0.5,
        "the pane comes back the {pane} it went away, and came back {}",
        width(&dock, LATTICE),
    );
    for (node, was) in [SPECTRAL, SETTINGS].iter().zip(now) {
        assert!(
            (width(&dock, *node) - was).abs() < 0.5,
            "{node:?} was not touched by the unfold: {was} became {}",
            width(&dock, *node),
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
    assert!((window - (1000.0 - (column - rail()))).abs() < 0.01);
    let window = frame(&mut folds, &mut dock, &mut dial, window);
    assert!((width(&dock, SETTINGS) - rail()).abs() < 0.01);
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
    let rails = rail() + style().separator.width + rail();
    let window = frame(&mut folds, &mut dock, &mut dial, 1000.0);
    assert!(
        (window - (1000.0 - (pair - rails))).abs() < 0.01,
        "the window loses the pair, less the two rails it leaves"
    );
    // The inner split divides what the fold hands it; only the fold itself
    // charges the window, or the pair would be paid for twice.
    let settled = frame(&mut folds, &mut dock, &mut dial, window);
    assert!((settled - window).abs() < 0.01, "one fold, one resize");
    assert!((width(&dock, LATTICE) - rail()).abs() < 0.01, "the lattice's own rail");
    assert!((width(&dock, SPECTRAL) - rail()).abs() < 0.01, "the analyzer's own rail");
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

/// A dock with every pane folded is a strip of rails, and the window comes
/// down to what that strip is worth.
///
/// A split is collapsed once both its children are, so folding the last pane
/// collapses the root — and the width a folded split gives up is its PARENT's
/// to take, which leaves a root fold nobody's. [`holds`] claims it, or the one
/// dock folded WHOLE would be the one dock nothing reads as folded at all: full
/// window width, with nothing drawn in it but rails.
#[test]
fn a_dock_folded_whole_is_a_strip_of_rails() {
    let mut dock = dock();
    let mut folds = Folds::default();
    let mut dial = Dial::default();
    let mut window = settle(&mut folds, &mut dock, &mut dial, 1000.0);
    for tab in [Tab::Lattice, Tab::Spectral, Tab::Tuning, Tab::Notes] {
        collapse(&mut dock, tab, true);
        window = settle(&mut folds, &mut dock, &mut dial, window);
    }
    // Three rails side by side: the two pictures, and the settings column as
    // one — its panes fold onto each other's tab bars, top to bottom.
    let separator = style().separator.width;
    let strip = 3.0 * rail() + 2.0 * separator;
    assert!(
        (window - strip).abs() < 1.0,
        "the folded dock came out {window:.1} wide, not the {strip:.1} its rails are worth",
    );
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
    // Settled, not one frame: a fold waits for the window it asked for before
    // its flags land ([`Wait`]), so the entry this is about does not exist yet
    // on the frame the click is caught. Asserting `is_empty` against a fold
    // that has not been recorded proves nothing about `clear` dropping it.
    let window = settle(&mut folds, &mut dock, &mut dial, 1000.0);
    assert!(!folds.is_empty(), "the fold is recorded before the reset throws it away");
    assert!((window + folds.clear(&dock, &style(), &dial, window) - 1000.0).abs() < 0.01);
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
    let owed = folds.clear(&dock, &style(), &dial, window);
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

/// A blob written while an entry still named a surface, and while a fraction
/// was what one held instead of a width, loads with both keys ignored.
///
/// This is the whole cost of retiring either field, and it is why retiring
/// them was affordable: `UiPersist` is one RON document, so a blob these
/// entries could not parse would cost the saved layout entire and not just its
/// folds. Serde passes over a key it does not know, so what is lost is the
/// value and never the entry — a fold recorded before the layout was points
/// comes back with no width to seed from, and opens at the rail width its
/// fraction in the tree says.
#[test]
fn a_fold_blob_still_carrying_the_retired_keys_loads_without_them() {
    let older = "([(surface:0,node:1,side:Right,width:487.0,window:1000.0),\
                   (surface:0,node:5,fraction:0.35)])";
    let loaded: Folds = ron::from_str(older).expect("an older blob still loads");
    assert_eq!(loaded.0.len(), 2, "both entries survive the keys they carry");
    assert_eq!(loaded.0[0].node, 1);
    assert!((loaded.0[0].width - 487.0).abs() < 0.01, "what it does still hold is read");
    assert_eq!(loaded.0[1].width, 0.0, "and a fraction seeds nothing");
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
    assert!((width(&dock, SPECTRAL) - rail()).abs() < 0.01, "the analyzer is the rail now");
    collapse(&mut dock, Tab::Spectral, false);
    let window = settle(&mut folds, &mut dock, &mut dial, window);
    assert!((window - 1000.0).abs() < 0.01, "the window is the one it started in");
    assert!((width(&dock, SPECTRAL) - analyzer).abs() < 0.01, "and the analyzer with it");
}

/// Two folds released in the same pass, one inside the other, each hand back
/// what they took: the outer hands its subtree a width the inner one then
/// divides, so it has to go first. Restored the other way round, the inner
/// fold's refund inflates what the outer measures itself against and the outer
/// pays back a NEGATIVE width, stranding the window a fold too narrow for good.
///
/// Folding the analyzer is where a pass does two at once: the fold its sibling
/// took is released and one at their parent is recorded in its place. Folding
/// the settings column releases THAT one onto the strip, and opening the panes
/// again walks the whole thing back.
///
/// Eight clicks, which is past the six
/// `every_round_trip_of_clicks_lands_where_it_started` can afford — so this is
/// the one round trip that passes through a dock folded WHOLE.
#[test]
fn two_folds_released_at_once_hand_back_what_each_took() {
    let mut dock = dock();
    let mut folds = Folds::default();
    let mut dial = Dial::default();
    // A settled frame before the click, as the editor always has: the
    // layout is dialled in at the window it is being drawn in.
    let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
    let tabs = [Tab::Lattice, Tab::Spectral, Tab::Tuning, Tab::Notes];
    let mut window = 1000.0;
    for tab in tabs {
        collapse(&mut dock, tab, true);
        window = settle(&mut folds, &mut dock, &mut dial, window);
    }
    for tab in tabs {
        collapse(&mut dock, tab, false);
        window = settle(&mut folds, &mut dock, &mut dial, window);
    }
    assert!(
        (window - 1000.0).abs() < 0.01,
        "the folds handed back {window:.1} of the 1000 they took",
    );
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
    assert!((width(&dock, LATTICE) - rail()).abs() < 0.01, "the rails are still rails");
    assert!((width(&dock, SPECTRAL) - rail()).abs() < 0.01);
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
        (width(&dock, LATTICE) - rail()).abs() < 0.01,
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
fn drag(dial: &mut Dial, dock: &mut DockState<Tab>, node: NodeIndex, delta: f32) {
    // A drag is a pointer doing something, which is what tells it from the
    // clamp egui_dock applies to every separator on every frame — and the
    // pointer travels exactly as far as the drag it is making, which is what
    // the boundary follows once it is held (see `Grip`).
    let at = dial.pointer.unwrap_or(0.0) + delta;
    dial.watch_pointer(true, Some(at));
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
    let mut nobody = Dial::default();
    for index in 0..dock[SurfaceIndex::main()].len() {
        let node = NodeIndex(index);
        if dock[SurfaceIndex::main()][node].is_parent() {
            drag(&mut nobody, dock, node, 0.0);
        }
    }
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
    drag(&mut dial, &mut dock, NodeIndex::root(), 40.0);
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
    drag(&mut dial, &mut dock, NodeIndex::root(), 40.0);
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
    drag(&mut dial, &mut dock, NodeIndex::root(), 40.0);
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
    assert!((width(&dock, SPECTRAL) - rail()).abs() < 0.01, "the rail is still a rail");
    assert!((asked - window).abs() < 0.01, "and a drag is not a resize: the window stays");
    // Held, rather than undone one frame later, which is the whole bug.
    let _ = frame(&mut folds, &mut dock, &mut dial, window);
    assert!(
        (width(&dock, PICTURES) - (pictures + 40.0)).abs() < 1.0,
        "the frame after should be where the drag left it",
    );
}

/// A separator the pointer wiggles back and forth stays under the pointer.
///
/// What a drag moves is points across a boundary, between the panes that are on
/// screen to trade them — so the separators drawn BETWEEN those panes are on
/// neither side of it. Counted into the side that holds them, they are handed
/// over again on every frame the pointer moves, whichever way it is going, and
/// the boundary walks out from under it a separator at a time: a second of
/// wiggling is 40 frames, and 40 points of daylight between the pointer and the
/// handle it is holding. Holding still leaks nothing, which is what makes this
/// the gesture that finds it.
///
/// Both ways round, since which side gains is whichever holds the more
/// separators — and the pair of leaves, which holds none either side and so
/// never drifted at all.
#[test]
fn a_separator_wiggled_back_and_forth_stays_under_the_pointer() {
    for (what, mut dock, node, fold) in [
        ("the settings boundary", dock(), NodeIndex::root(), None),
        ("the picture boundary", dock(), PICTURES, None),
        ("a row's leftmost boundary", row(), NodeIndex::root(), None),
        ("the settings boundary, analyzer folded", dock(), NodeIndex::root(), Some(Tab::Spectral)),
    ] {
        let mut folds = Folds::default();
        let mut window = Window::new(1000.0);
        window.settle(&mut folds, &mut dock);
        if let Some(tab) = fold {
            collapse(&mut dock, tab, true);
            window.settle(&mut folds, &mut dock);
        }
        // Where the handle is drawn, which is what the pointer is holding.
        let at = |dock: &DockState<Tab>| {
            let rect = dock[SurfaceIndex::main()][node].rect().expect("the split is on screen");
            rect.left() + rect.width() * fraction(dock, node)
        };
        let start = at(&dock);
        let (mut pointer, mut worst) = (start, 0.0f32);
        for step in 0..40 {
            let delta = if step % 2 == 0 { 8.0 } else { -8.0 };
            pointer += delta;
            drag(&mut window.dial, &mut dock, node, delta);
            window.frame(&mut folds, &mut dock);
            worst = worst.max((at(&dock) - pointer).abs());
        }
        assert!(worst < 0.5, "{what} came {worst:.1}pt out from under the pointer");
        assert!((at(&dock) - start).abs() < 0.5, "{what} did not end where it started");
    }
}

/// A boundary dragged past a floor does not set off the moment the pointer
/// turns around: the points the drag could not spend are still owed, and the
/// pointer has to come back past the floor before the pane grows again.
///
/// Which is the same property as the wiggle above, at the one place a drag is
/// guaranteed to lose points — and the reason the boundary follows the pointer's
/// travel from where it took hold rather than a sum of per-frame steps. A sum
/// has no memory of what a clamp refused, so the overshoot is gone the moment it
/// is made: push 300 points past the floor and the handle comes away 300 points
/// from the hand still holding it, for as long as the gesture lasts.
#[test]
fn a_boundary_dragged_past_a_floor_waits_for_the_pointer_to_come_back() {
    // Both ways, since each side of the boundary has a floor of its own.
    for step in [-40.0f32, 40.0] {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut window = Window::new(1000.0);
        window.settle(&mut folds, &mut dock);
        let node = NodeIndex::root();
        let at = |dock: &DockState<Tab>| {
            let rect = dock[SurfaceIndex::main()][node].rect().expect("the split is on screen");
            rect.left() + rect.width() * fraction(dock, node)
        };
        let start = at(&dock);
        // 800 points that way, which is further than the panes on it can go.
        for _ in 0..20 {
            drag(&mut window.dial, &mut dock, node, step);
            window.frame(&mut folds, &mut dock);
        }
        let stopped = at(&dock);
        let travelled = (stopped - start).abs();
        // It accumulated more than a single step before the floor caught it,
        // and stopped well short of the 800 pushed. Both bounds are stated
        // against numbers this test already holds, because what the boundary
        // CAN travel is the shrinking pane's starting width less
        // `separator.extra` — the two move together point for point, so a
        // literal here fails on a tab-bar retune and fails naming the fold
        // layout, which is the one thing that did not change.
        assert!(
            travelled > step.abs(),
            "the boundary moved {travelled:.0}pt before stopping, less than one {:.0}pt step",
            step.abs(),
        );
        assert!(travelled < 700.0, "the boundary should have stopped, and moved {travelled:.0}pt");
        // Back, but not yet as far as the boundary is.
        for _ in 0..2 {
            drag(&mut window.dial, &mut dock, node, -step);
            window.frame(&mut folds, &mut dock);
        }
        assert!(
            (at(&dock) - stopped).abs() < 0.5,
            "the handle set off 80pt into a {:.0}pt overshoot",
            800.0 - travelled,
        );
        // All the way back is all the way back.
        for _ in 0..18 {
            drag(&mut window.dial, &mut dock, node, -step);
            window.frame(&mut folds, &mut dock);
        }
        assert!(
            (at(&dock) - start).abs() < 0.5,
            "the boundary came back to {:.0}, not the {start:.0} the gesture found it at",
            at(&dock),
        );
    }
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
    drag(&mut dial, &mut dock, NodeIndex::root(), 40.0);
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

/// A pane is never DRAWN collapsed at more than a rail's width.
///
/// A fold is a two-step — one frame decides what to ask the window for, the
/// next is laid out in the window it was given — and a collapsed flag that
/// lands on the first of those is a pane with its body gone and its whole
/// column kept: a wide empty hole for exactly one frame, worth a fifth of the
/// editor where the Spectral pane is what folded (207 of 1000).
///
/// Deferring the layout write does not reach it. The hole is what a collapsed
/// flag draws at an un-narrowed fraction, and the fraction is the thing being
/// deferred — so the flag is what has to wait ([`Wait`]). Held, the pane stays
/// open and fully drawn for that frame, which is what the frame before it
/// showed, and folds on the frame whose window fits the rail.
#[test]
fn a_pane_is_never_drawn_collapsed_at_more_than_a_rail() {
    let mut dock = dock();
    let mut window = Window::new(1000.0);
    let mut folds = Folds::default();
    window.settle(&mut folds, &mut dock);
    // egui_dock's own collapse button, which it draws from inside `show` — so
    // the flag arrives between one fold pass and the next.
    let neighbour = width(&dock, LATTICE);
    collapse(&mut dock, Tab::Spectral, true);
    for frame in 0..6 {
        window.frame(&mut folds, &mut dock);
        let path = dock.find_tab(&Tab::Spectral).expect("docked");
        let node = &dock[path.surface][path.node];
        let width = node.rect().expect("on screen").width();
        assert!(
            !node.is_collapsed() || width < 40.0,
            "frame {frame}: a collapsed pane is {width} wide, not a rail"
        );
    }
    // And it did fold, rather than passing by never folding at all.
    let path = dock.find_tab(&Tab::Spectral).expect("docked");
    assert!(dock[path.surface][path.node].is_collapsed(), "the pane never folded");
    // The pane beside it never moved. Writing the narrowed layout on the
    // asking frame would keep the folded pane at a rail and satisfy the loop
    // above, while the neighbours swell into the width the fold freed and give
    // it back once the window lands — the other one-frame artifact, and the
    // one the loop cannot see.
    assert!(
        (width(&dock, LATTICE) - neighbour).abs() < 0.01,
        "the pane beside the fold moved from {neighbour} to {}",
        width(&dock, LATTICE),
    );
}

/// A downward fold lands on the frame it is clicked. It takes no width off the
/// window, so there is nothing for it to wait for.
///
/// `Folds` is for the sideways folds alone — `folded_side` reads horizontal
/// splits and nothing else — and egui_dock does the whole of a vertical one.
/// A hold armed for a gesture that moves no boundary is a frame of latency
/// bought for nothing, and the settings column's Notes/Console bar, which
/// ships folded, is the arrow it would be bought on.
///
/// The settled frame first is the whole of why this can fail: a fresh `Dial`
/// has no flags, and the length guard on the snapshot makes the detector
/// short-circuit — which is why the sibling test above cannot see it.
#[test]
fn a_pane_folded_downwards_lands_on_the_frame_it_is_clicked() {
    let mut dock = dock();
    let mut folds = Folds::default();
    let mut dial = Dial::default();
    let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
    collapse(&mut dock, Tab::Tuning, true);
    let asked = frame(&mut folds, &mut dock, &mut dial, 1000.0);
    assert!(collapsed_tab(&dock, Tab::Tuning), "a downward fold waited on a window");
    assert!((asked - 1000.0).abs() < 0.01, "and asked the window for {asked}, not nothing");
}

/// A hold waits for the window it ASKED for, not for any move at all.
///
/// Both directions, because the two are told apart by one flag ([`Wait::shuts`])
/// and a shell that answers every ask exactly cannot tell a right answer from a
/// wrong one — every arrival satisfies both comparisons at once. What separates
/// them is a window that moves the way the gesture did NOT ask: a host resize,
/// a display change, a drag landing in the held frame.
#[test]
fn a_hold_waits_for_the_window_it_asked_for_and_not_the_other_way() {
    // A fold asks for a narrower window. A wider one is not it.
    {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        collapse(&mut dock, Tab::Spectral, true);
        let asked = frame(&mut folds, &mut dock, &mut dial, 1000.0);
        assert!(asked < 999.0, "the fold asks for a narrower window, asked {asked}");
        let _ = frame(&mut folds, &mut dock, &mut dial, 1100.0);
        assert!(
            !collapsed_tab(&dock, Tab::Spectral),
            "the fold landed in a window wider than the one it was drawn in",
        );
    }

    // And an unfold asks for a wider one, where a narrower window is not it.
    {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        collapse(&mut dock, Tab::Spectral, true);
        let window = settle(&mut folds, &mut dock, &mut dial, 1000.0);
        collapse(&mut dock, Tab::Spectral, false);
        let asked = frame(&mut folds, &mut dock, &mut dial, window);
        assert!(asked > window + 1.0, "the unfold asks for a wider window, asked {asked}");
        let _ = frame(&mut folds, &mut dock, &mut dial, window - 100.0);
        assert!(
            collapsed_tab(&dock, Tab::Spectral),
            "the unfold landed in a window narrower than the one it was drawn in",
        );
    }
}

/// What a fold banks as the window it was taken at is the window the PANE was
/// last open in — never the narrower one an unfold happens to be landing in.
///
/// `Fold::window` is the ceiling a later unfold may ask the host for, and it is
/// the only record of that ceiling to survive a persist blob ([`Dial::widest`]
/// is runtime-only). An unfold's landing frame creates entries too, wherever a
/// pair that was folded whole becomes folded on one side — and priced against
/// the window the unfold STARTED in, every such landing ratchets the ceiling
/// down, so a pane reopened in a later session comes back short and takes the
/// difference out of its neighbour instead of out of the window.
#[test]
fn an_unfold_landing_does_not_ratchet_down_the_ceiling_a_fold_banked() {
    let mut dock = dock();
    let mut window = Window::new(1400.0);
    let mut folds = Folds::default();
    window.settle(&mut folds, &mut dock);
    collapse(&mut dock, Tab::Lattice, true);
    window.settle(&mut folds, &mut dock);
    collapse(&mut dock, Tab::Spectral, true);
    window.settle(&mut folds, &mut dock);
    // Both pictures are rails now, so the pair is folded whole and holds no
    // side. Opening one gives it a side again, which is a fold entry made on
    // an UNFOLD's landing frame.
    let narrow = window.size;
    collapse(&mut dock, Tab::Lattice, false);
    window.settle(&mut folds, &mut dock);
    let pair = folds.0.iter().find(|fold| fold.node == PICTURES.0).expect("the pair holds a side");
    assert!(
        pair.window > narrow + 1.0,
        "the pair banked {}, the window the unfold started in ({narrow})",
        pair.window,
    );
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
        let width = dock[path.surface][path.node].rect().expect("on screen").width();
        assert!((width - rail()).abs() < 0.01, "{tab:?} came out {width} wide, not a rail");
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
            drag(&mut window.dial, &mut dock, NodeIndex::root(), delta);
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

/// An unfold waits for its window, whoever did it. egui_dock re-derives its
/// own fractions inside `show` the moment the flag comes off, so a pane opened
/// against the window it is LEAVING is laid out in that window and every
/// boundary on screen jumps inward for the frame (see [`Wait`]).
///
/// Driven through `collapse`, which is egui_dock's own expand: that is the
/// writer the hold has to catch, and the rail arrow is only the other door
/// into the same place.
#[test]
fn an_unfold_stays_shut_until_the_window_can_hold_the_pane() {
    let mut dock = dock();
    let mut folds = Folds::default();
    let mut dial = Dial::default();
    let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
    let open_at = width(&dock, LATTICE);
    collapse(&mut dock, Tab::Lattice, true);
    let narrow = settle(&mut folds, &mut dock, &mut dial, 1000.0);

    // The unfold, as egui_dock does it from inside `show`.
    collapse(&mut dock, Tab::Lattice, false);
    let asked = frame(&mut folds, &mut dock, &mut dial, narrow);
    assert!(asked > narrow + 0.01, "the click asks the window for the pane's width");
    assert!(
        collapsed(&dock[SurfaceIndex::main()], LATTICE),
        "and the pane is still a rail, so the dock lays this frame out without it"
    );

    // The frame that has the window the ask bought.
    let _ = frame(&mut folds, &mut dock, &mut dial, asked);
    assert!(
        !collapsed(&dock[SurfaceIndex::main()], LATTICE),
        "the flag comes off on the frame wide enough to hold the pane"
    );
    assert!(
        (width(&dock, LATTICE) - open_at).abs() < 1.0,
        "and the pane is back at the width it folded from, in one step",
    );
}

/// A window that will not grow must not leave the pane shut forever. The wait
/// is for a resize that was asked for, so a resize that never comes ends it
/// just as arriving would — the pane opens into whatever width there is, which
/// is what the rest of the pass already does with a refusal.
#[test]
fn an_unfold_the_window_refuses_opens_anyway() {
    let mut dock = dock();
    let mut folds = Folds::default();
    let mut dial = Dial::default();
    let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
    collapse(&mut dock, Tab::Lattice, true);
    let narrow = settle(&mut folds, &mut dock, &mut dial, 1000.0);

    collapse(&mut dock, Tab::Lattice, false);
    let _ = frame(&mut folds, &mut dock, &mut dial, narrow);
    // The host holds the window where it is, so the next frame is laid out at
    // the width the last one was.
    let _ = frame(&mut folds, &mut dock, &mut dial, narrow);
    assert!(
        !collapsed(&dock[SurfaceIndex::main()], LATTICE),
        "a refused resize opens the pane rather than holding it shut"
    );
}

/// A shell that can only hand back whole points, which is what `editor.rs`
/// does with the ask (`(width + change).round()`, and nothing under half a
/// point asked at all).
#[must_use]
fn frame_rounded(folds: &mut Folds, dock: &mut DockState<Tab>, dial: &mut Dial, width: f32) -> f32 {
    let change = folds.apply(dock, &style(), width, 0.0, dial);
    lay_out(dock, width);
    if change.abs() < 0.5 {
        width
    } else {
        (width + change).round()
    }
}

/// Folding a pane must not move the pane beside it, even by the fraction of a
/// point the window cannot be asked for.
///
/// The layout wants a fractional window and can only be given a whole one, so
/// there is always a remainder; shared out over the panes it lands on every
/// boundary at once, and the leftmost pane sliding is what the eye actually
/// reads. Several starting widths because the remainder's SIGN follows the
/// window: one width alone can be right by luck.
#[test]
fn folding_a_pane_does_not_move_its_neighbour_in_a_whole_point_window() {
    for start in [1000.0_f32, 1001.0, 1237.0, 1400.5] {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut dial = Dial::default();
        let mut window = start;
        for _ in 0..3 {
            window = frame_rounded(&mut folds, &mut dock, &mut dial, window);
        }
        let before = width(&dock, LATTICE);
        collapse(&mut dock, Tab::Spectral, true);
        for _ in 0..4 {
            window = frame_rounded(&mut folds, &mut dock, &mut dial, window);
        }
        let after = width(&dock, LATTICE);
        assert!(
            (after - before).abs() < 0.01,
            "folding the analyzer at {start} moved the lattice from {before} to {after}",
        );
    }
}

/// Resetting the layout while a pane is folded must not fold it again.
///
/// The reset swaps the whole dock, and folding never changes the tree's SHAPE,
/// so the flags left over from the old one fit the new one exactly and read as
/// an unfold nobody did — putting the pane the reset just opened straight back
/// for a frame.
#[test]
fn resetting_the_layout_does_not_re_fold_the_pane_it_just_opened() {
    let mut dock = dock();
    let mut folds = Folds::default();
    let mut dial = Dial::default();
    let _ = frame(&mut folds, &mut dock, &mut dial, 1000.0);
    collapse(&mut dock, Tab::Lattice, true);
    let narrow = settle(&mut folds, &mut dock, &mut dial, 1000.0);

    // The reset, as `root_ui` does it: a fresh dock, the folds given up, and
    // the dial told the tree it was reading is gone.
    let mut dock = super::tests::dock();
    folds = Folds::default();
    dial.forget();
    let _ = frame(&mut folds, &mut dock, &mut dial, narrow);
    assert!(
        !collapsed(&dock[SurfaceIndex::main()], LATTICE),
        "the reset layout has every pane open, on the first frame and not the second"
    );
}

/// A separator resizes the two panes it is DRAWN between and leaves every other
/// pane exactly where it was.
///
/// The plugin's own arrangement is the case: the settings column's boundary has
/// the analyzer on the other side of it and the lattice further along, and what
/// the tree says is that the boundary divides the settings column from the
/// PICTURE PAIR. Shared out over that pair in proportion, 100 points handed to
/// the settings column come 70 off the lattice and 30 off the analyzer — the
/// picture re-laid-out for a gesture aimed at one edge of it, and the pane the
/// user was pointing at moving least of the two.
///
/// Both ways, since each direction is a different pane's floor to stop at — and
/// short of either, since a drag that lands ON a floor is testing the clamp
/// rather than the share-out. 75 points leaves the analyzer 30 clear of its own.
#[test]
fn a_separator_moves_the_panes_it_is_drawn_between_and_nothing_else() {
    for step in [-15.0f32, 15.0] {
        let mut dock = dock();
        let mut folds = Folds::default();
        let mut window = Window::new(1000.0);
        window.settle(&mut folds, &mut dock);
        let (lattice, spectral, settings) =
            (width(&dock, LATTICE), width(&dock, SPECTRAL), width(&dock, SETTINGS));
        for _ in 0..5 {
            drag(&mut window.dial, &mut dock, NodeIndex::root(), step);
            window.frame(&mut folds, &mut dock);
        }
        let moved = step * 5.0;
        assert!(
            (width(&dock, LATTICE) - lattice).abs() < 1.0,
            "the lattice is not on either side of the separator, and moved {:.1}pt",
            width(&dock, LATTICE) - lattice,
        );
        assert!(
            (width(&dock, SPECTRAL) - (spectral + moved)).abs() < 1.0,
            "the analyzer is what the separator is drawn against, and moved {:.1}pt of {moved:.1}",
            width(&dock, SPECTRAL) - spectral,
        );
        assert!(
            (width(&dock, SETTINGS) - (settings - moved)).abs() < 1.0,
            "the settings column should have paid for it, and moved {:.1}pt",
            width(&dock, SETTINGS) - settings,
        );
    }
}

/// At the floor of the pane it is drawn against, the boundary STOPS — it does
/// not carry on by taking the width off the pane behind that one.
///
/// Pushing through is what a row of sashes does elsewhere, and here it is the
/// rule read backwards: the analyzer, already as narrow as it goes, would slide
/// sideways at a fixed width while the LATTICE paid for a gesture aimed at the
/// settings edge. From the outside that is the other handle moving on its own,
/// which is what it was reported as.
///
/// Stopping costs the gesture nothing: the boundary follows the pointer's travel
/// from where it took hold ([`Grip`]), so the points a floor refuses are owed
/// and repaid on the way back rather than lost —
/// `a_boundary_dragged_past_a_floor_waits_for_the_pointer_to_come_back` is that
/// property, and this is the floor it now stops at.
#[test]
fn a_drag_stops_at_the_floor_of_the_pane_it_is_drawn_against() {
    let mut dock = dock();
    let mut folds = Folds::default();
    let mut window = Window::new(1000.0);
    window.settle(&mut folds, &mut dock);
    let (lattice, spectral) = (width(&dock, LATTICE), width(&dock, SPECTRAL));
    // A leaf's floor, as `drags` prices it off egui_dock's own separator limit.
    let floor = style().separator.extra - style().separator.width * 0.5;
    // Further than the analyzer can pay for, and well short of what it and the
    // lattice could pay for between them.
    let travel = (spectral - floor) + 90.0;
    for _ in 0..(travel / 20.0).ceil() as usize {
        drag(&mut window.dial, &mut dock, NodeIndex::root(), -20.0);
        window.frame(&mut folds, &mut dock);
    }
    assert!(
        (width(&dock, SPECTRAL) - floor).abs() < 1.0,
        "the analyzer should be sitting on its floor, and is {:.1}",
        width(&dock, SPECTRAL),
    );
    assert!(
        (width(&dock, LATTICE) - lattice).abs() < 1.0,
        "the lattice is not what the separator is drawn against, and gave up {:.1}pt",
        lattice - width(&dock, LATTICE),
    );
}

/// A pair either side of one boundary, so both sides have a pane BEHIND the one
/// the separator is drawn against — the shape that can tell the two rules apart
/// in both directions at once. The plugin's own dock is a pair against a single
/// column, where the right-hand side agrees whatever the rule is.
fn pairs() -> DockState<Tab> {
    let mut dock = DockState::new(vec![Tab::Lattice]);
    let surface = dock.main_surface_mut();
    let [left, right] = surface.split_right(NodeIndex::root(), 0.5, vec![Tab::Notes]);
    surface.split_right(left, 0.5, vec![Tab::Spectral]);
    surface.split_right(right, 0.5, vec![Tab::Tuning]);
    dock
}

/// Four panes, one boundary: the two it is drawn between move and the outer two
/// do not, on both sides of it. Both directions, since which of the two facing
/// panes is being squeezed swaps with the direction.
#[test]
fn a_separator_between_two_pairs_moves_the_inner_pane_of_each() {
    for step in [-20.0f32, 20.0] {
        let mut dock = pairs();
        let mut folds = Folds::default();
        let mut window = Window::new(1000.0);
        window.settle(&mut folds, &mut dock);
        let at = |dock: &DockState<Tab>, tab| {
            width(dock, dock.find_tab(&tab).expect("the tab is docked").node)
        };
        // Left to right, which is not the order the tree nests them in.
        let row = [Tab::Lattice, Tab::Spectral, Tab::Notes, Tab::Tuning];
        let before = row.map(|tab| at(&dock, tab));
        for _ in 0..3 {
            drag(&mut window.dial, &mut dock, NodeIndex::root(), step);
            window.frame(&mut folds, &mut dock);
        }
        let after = row.map(|tab| at(&dock, tab));
        let moved: Vec<f32> = after.iter().zip(&before).map(|(a, b)| a - b).collect();
        let want = step * 3.0;
        assert!(moved[0].abs() < 1.0, "the outer left pane moved {:.1}pt", moved[0]);
        assert!(
            (moved[1] - want).abs() < 1.0,
            "the pane left of the separator moved {:.1}pt of the {want}",
            moved[1],
        );
        assert!((moved[2] + want).abs() < 1.0, "the pane right of it moved {:.1}pt", moved[2]);
        assert!(moved[3].abs() < 1.0, "the outer right pane moved {:.1}pt", moved[3]);
    }
}

/// A column with a side-by-side pair in it, which egui_dock lets a user build by
/// dropping one tab beside another inside a column.
///
/// The shape that tells a vertical split's room from its floor. Every other
/// fixture here bottoms out at a leaf or at an all-leaf column, where the two
/// are the same number.
fn column_with_a_pair() -> DockState<Tab> {
    let mut dock = DockState::new(vec![Tab::Lattice]);
    let surface = dock.main_surface_mut();
    let [_, column] = surface.split_right(NodeIndex::root(), 0.5, vec![Tab::Tuning]);
    let [_, bottom] = surface.split_below(column, 0.5, vec![Tab::Notes]);
    surface.split_right(bottom, 0.5, vec![Tab::Display]);
    dock
}

/// A column squeezed by the separator down its side moves AS ONE, and stops
/// where the narrowest row in it stops.
///
/// Both rows of a column are drawn at the same width and both reach the
/// boundary, so a drag that hands each of them the same points is only correct
/// while each can take them. Where one row is a PAIR, the pane facing the
/// boundary is half a row, and the drag runs out there long before the row does
/// — so a column priced at what its own floor allows promises points that the
/// pair's near pane cannot pay. It stops at its floor, the leftovers go
/// nowhere, and the two rows come out different widths: the pane BEHIND the
/// near one wears the difference, which is the whole thing this separator is
/// not supposed to touch.
#[test]
fn a_column_with_a_pair_in_it_is_squeezed_no_further_than_the_pair_allows() {
    let mut dock = column_with_a_pair();
    let mut folds = Folds::default();
    let mut window = Window::new(1000.0);
    window.settle(&mut folds, &mut dock);
    let node = |dock: &DockState<Tab>, tab| dock.find_tab(&tab).expect("the tab is docked").node;
    let at = |dock: &DockState<Tab>, tab| width(dock, node(dock, tab));
    let floor = style().separator.extra - style().separator.width * 0.5;
    let behind = at(&dock, Tab::Display);
    // Rightward, which is the column shrinking: far enough that the pair's near
    // pane reaches its floor well before the column's own would.
    for _ in 0..15 {
        drag(&mut window.dial, &mut dock, NodeIndex::root(), 20.0);
        window.frame(&mut folds, &mut dock);
    }
    assert!(
        at(&dock, Tab::Notes) > floor - 1.0,
        "the pane facing the boundary is drawn at {:.1}, under its {floor} floor",
        at(&dock, Tab::Notes),
    );
    assert!(
        (at(&dock, Tab::Display) - behind).abs() < 1.0,
        "the pane behind it is not what the separator divides, and moved {:.1}pt",
        at(&dock, Tab::Display) - behind,
    );
    // The column is one column, however deep either row nests.
    let (top, bottom) = (at(&dock, Tab::Tuning), node(&dock, Tab::Notes).parent().unwrap());
    assert!(
        (top - width(&dock, bottom)).abs() < 1.0,
        "the column's rows came out {top:.1} and {:.1} wide",
        width(&dock, bottom),
    );
}
