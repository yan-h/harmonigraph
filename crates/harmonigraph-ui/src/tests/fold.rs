//! Folding panes sideways: what a fold banks, which separators still
//! resize, and the rails a folded subtree leaves behind.

use crate::*;
use harmonigraph_render::wgpu::TextureFormat;
use super::harness::*;

/// The layout opens with Notes and Console folded to their tab bar, and with
/// nothing else folded.
///
/// Both are read on demand — Notes restates what the lattice is already
/// drawing, Console is a diagnostic — and open they take 45% of the settings
/// column, the half the settings themselves want. The collapse arrow on the
/// folded bar brings either back at the size it went away, so this is a
/// starting point rather than a decision taken away.
#[test]
fn the_default_layout_opens_with_the_two_readout_panes_folded() {
    let dock = default_dock();
    let folded = |tab: panes::Tab| {
        let path = dock.find_tab(&tab).expect("docked by default");
        let egui_dock::Node::Leaf(leaf) = &dock[path.surface][path.node] else {
            panic!("{tab:?} should live in a leaf");
        };
        leaf.collapsed
    };
    assert!(folded(panes::Tab::Notes), "Notes should open folded");
    assert!(folded(panes::Tab::Console), "Console should open folded");
    for tab in [panes::Tab::Lattice, panes::Tab::Spectral, panes::Tab::Tuning] {
        assert!(!folded(tab), "{tab:?} should open on screen");
    }
}

/// The width a sideways fold gives up reaches the shell, through the whole
/// path the editor uses: `root_ui` measures the dock area from THIS frame's
/// `Ui`, the fold takes the pane's width out of it, and what is left over is
/// banked for the shell to spend on the window.
///
/// Driven through the real dock because that is the only place the area is
/// read from a live `Ui` rather than from the tree — `fold`'s own tests hand
/// `apply` a width directly, so a `root_ui` that measured the wrong rectangle,
/// or never asked at all, would leave every one of them green.
#[test]
fn collapsing_a_pane_banks_the_width_it_gave_up() {
    let mut h = DockHarness::new();
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    h.settle(&mut state);
    assert_eq!(state.take_window_width_change(), None, "an idle frame asks for nothing");

    let path = state.dock.find_tab(&panes::Tab::Lattice).expect("the lattice is docked");
    let pane = state.dock[path.surface][path.node].rect().expect("laid out").width();
    state.dock[path.surface][path.node].set_collapsed(true);
    h.frame(&mut state, vec![]);

    // A rail's worth of the pane stays behind; the rest comes off the window.
    let rail = theme::dock_style(&egui::Style::default(), 1.0).tab_bar.height;
    let change = state.take_window_width_change().expect("the fold asks for a narrower window");
    assert!(
        (change + (pane - rail)).abs() < 1.0,
        "a {pane}pt pane leaving a {rail}pt rail should ask for {}, asked {change}",
        rail - pane,
    );
    h.frame(&mut state, vec![]);
    assert_eq!(state.take_window_width_change(), None, "and asks exactly once");
}

/// Every separator a fold has pinned resizes the two nearest open panes across
/// it — the one the rail sits against, and, with two panes collapsed side by
/// side, the one between the rails as well. None of them can move what is
/// immediately either side of it (a rail is a fixed number of points), so they
/// all move the boundary the user is actually pointing at, with the rails
/// travelling along at their own width.
///
/// Driven through the real dock, because the handles have to WIN the pointer:
/// egui_dock draws its own separator in each of those places and keeps it live,
/// and what puts these over them is that `fold::paint` runs after the dock.
#[test]
fn every_separator_a_fold_has_pinned_resizes_the_open_panes_across_it() {
    let row = [
        panes::Tab::Lattice,
        panes::Tab::Spectral,
        panes::Tab::Tuning,
        panes::Tab::Notes,
        panes::Tab::Panel,
    ];
    // Two collapsed panes and then three, since each rail nests the next fold one
    // split deeper and the drag has to reach out past all of them.
    for rails in 2..=3usize {
        // Exactly one open pane on each side of the rails, so the drag has one
        // payer rather than a share-out among several.
        let row = &row[..rails + 2];
        // One run per handle and direction: a drag settles the layout it was
        // made in, and a handle that only answers one way is half a handle.
        for handle in 0..=rails {
            for delta in [45.0f32, -45.0] {
                let mut h = DockHarness::new();
                let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
                state.dock = a_row_of(row);
                h.settle(&mut state);
                for tab in &row[1..=rails] {
                    let _ = h.collapse_click(&mut state, *tab);
                }
                let _ = h.settle_folds(&mut state);

                let before: Vec<f32> = row.iter().map(|tab| pane_width(&state, *tab)).collect();
                let (left, right) =
                    (pane_rect(&state, row[handle]), pane_rect(&state, row[handle + 1]));
                let at = egui::pos2((left.right() + right.left()) * 0.5, left.center().y);
                h.frame(&mut state, vec![egui::Event::PointerMoved(at)]);
                h.frame(&mut state, vec![egui::Event::PointerMoved(at), press(at, true)]);
                let target = at + egui::vec2(delta, 0.0);
                h.frame(&mut state, vec![egui::Event::PointerMoved(target)]);
                h.frame(&mut state, vec![press(target, false)]);
                let _ = h.settle_folds(&mut state);
                let after: Vec<f32> = row.iter().map(|tab| pane_width(&state, *tab)).collect();

                let moved = |i: usize| after[i] - before[i];
                let what = format!("{rails} rails, handle {handle}, {delta:+}");
                assert!(
                    (moved(0) - delta).abs() < 1.5,
                    "{what}: the open pane on the left should have taken the drag, and took {}",
                    moved(0),
                );
                let last = rails + 1;
                assert!(
                    (moved(last) + delta).abs() < 1.5,
                    "{what}: the open pane on the right should have paid it, and paid {}",
                    -moved(last),
                );
                for (rail, tab) in row.iter().enumerate().take(last).skip(1) {
                    assert!(
                        moved(rail).abs() < 0.6,
                        "{what}: {tab:?} is a rail and should not have moved, and moved {}",
                        moved(rail),
                    );
                }
            }
        }
    }
}

/// The layout that ships, with the analyzer folded: both handles beside its rail
/// resize the lattice against the settings column, which is the pane on the other
/// side of the boundary those handles stand on. The column is a VERTICAL split, so
/// the drag passes out through a shape the synthetic rows above do not have.
#[test]
fn both_handles_beside_a_rail_resize_the_panes_around_it_in_the_default_layout() {
    for handle in [-2.0f32, 2.0] {
        let mut h = DockHarness::new();
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        h.settle(&mut state);
        let _ = h.collapse_click(&mut state, panes::Tab::Spectral);
        let _ = h.settle_folds(&mut state);
        let rail = pane_rect(&state, panes::Tab::Spectral);
        let before =
            (pane_width(&state, panes::Tab::Lattice), pane_width(&state, panes::Tab::Tuning));

        // Just inside the rail's left edge, then just outside its right one.
        let at = match handle < 0.0 {
            true => egui::pos2(rail.left() + handle, rail.center().y),
            false => egui::pos2(rail.right() + handle, rail.center().y),
        };
        h.frame(&mut state, vec![egui::Event::PointerMoved(at)]);
        h.frame(&mut state, vec![egui::Event::PointerMoved(at), press(at, true)]);
        let target = at + egui::vec2(50.0, 0.0);
        h.frame(&mut state, vec![egui::Event::PointerMoved(target)]);
        h.frame(&mut state, vec![press(target, false)]);
        let _ = h.settle_folds(&mut state);

        let after =
            (pane_width(&state, panes::Tab::Lattice), pane_width(&state, panes::Tab::Tuning));
        let which = if handle < 0.0 { "left of the rail" } else { "right of it" };
        assert!(
            (after.0 - (before.0 + 50.0)).abs() < 1.5,
            "{which}: the lattice should have taken the 50, and moved {}",
            after.0 - before.0,
        );
        assert!(
            (after.1 - (before.1 - 50.0)).abs() < 1.5,
            "{which}: the settings column should have paid it, and moved {}",
            after.1 - before.1,
        );
        assert!(
            collapsed(&state, panes::Tab::Spectral),
            "{which}: and the analyzer should still be folded — this was a resize",
        );
    }
}

/// No pane is dragged below its floor, wherever it sits in the tree.
///
/// egui_dock's own limit holds apart the two children of whichever split is being
/// dragged, and nothing deeper in either of them: dragging the settings boundary
/// outward used to leave the picture pair at one pane's floor with the two panes
/// inside it sharing that between them, 71 points and 27. A rail is exempt, being
/// what a pane folds to.
#[test]
fn no_pane_is_dragged_below_its_floor_wherever_it_sits() {
    let style = theme::dock_style(&egui::Style::default(), 1.0);
    let floor = style.separator.extra - style.separator.width * 0.5;
    let panes = [panes::Tab::Lattice, panes::Tab::Spectral, panes::Tab::Tuning];
    for fold in [None, Some(panes::Tab::Spectral)] {
        for (grab, delta) in [
            (panes::Tab::Spectral, 900.0f32),
            (panes::Tab::Tuning, 900.0),
            (panes::Tab::Tuning, -900.0),
        ] {
            let mut h = DockHarness::new();
            let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
            h.settle(&mut state);
            if let Some(tab) = fold {
                let _ = h.collapse_click(&mut state, tab);
                let _ = h.settle_folds(&mut state);
            }
            // The separator on the left of `grab`, dragged past every floor there
            // is — egui_dock's clamp and ours both stop it somewhere.
            let at = handle_left_of(&state, grab, &style);
            h.frame(&mut state, vec![egui::Event::PointerMoved(at)]);
            h.frame(&mut state, vec![egui::Event::PointerMoved(at), press(at, true)]);
            let target = at + egui::vec2(delta, 0.0);
            h.frame(&mut state, vec![egui::Event::PointerMoved(target)]);
            h.frame(&mut state, vec![egui::Event::PointerMoved(target)]);
            h.frame(&mut state, vec![press(target, false)]);
            let _ = h.settle_folds(&mut state);

            let what = format!("{fold:?} folded, {grab:?} boundary, {delta:+}");
            for tab in panes {
                let width = pane_width(&state, tab);
                if collapsed(&state, tab) {
                    assert!(
                        (width - style.tab_bar.height).abs() < 1.0,
                        "{what}: {tab:?} is a rail and should be a tab bar wide, and is {width}",
                    );
                    continue;
                }
                assert!(
                    width >= floor - 1.0,
                    "{what}: {tab:?} came out {width} wide, under the {floor} floor",
                );
            }
        }
    }
}

/// Whether any separator on screen is painted the colour a dragged one takes.
/// Tall and thin is what tells a separator from every other rectangle in the
/// editor; the colour is what the question is about.
fn a_bar_is_lit(out: &egui::FullOutput, style: &egui_dock::Style) -> bool {
    out.shapes.iter().any(|cs| match &cs.shape {
        egui::Shape::Rect(r) => {
            r.fill == style.separator.color_dragged
                && r.rect.width() < 12.0
                && r.rect.height() > 400.0
        }
        _ => false,
    })
}

/// Press on the boundary left of the settings column and drag it 60pt left,
/// which is a resize in progress by the time this returns.
fn start_dragging_the_settings_boundary(
    h: &mut DockHarness,
    state: &mut SharedState,
    style: &egui_dock::Style,
) -> egui::Pos2 {
    h.settle(state);
    let at = handle_left_of(state, panes::Tab::Tuning, style);
    h.frame(state, vec![egui::Event::PointerMoved(at)]);
    h.frame(state, vec![egui::Event::PointerMoved(at), press(at, true)]);
    let target = at + egui::vec2(-60.0, 0.0);
    h.frame(state, vec![egui::Event::PointerMoved(target)]);
    target
}

/// The bar being dragged stays lit while the pointer is off the window.
///
/// A plugin editor is small and a separator is often near its edge, so a resize
/// that carries on past that edge is an ordinary one: the pane goes on following
/// the pointer, which is what a drag names (see `fold::Grip`). What egui_dock
/// lights the bar from is its OWN drag, and the shell has to end that one the
/// moment the pointer leaves — a release delivered elsewhere strands it, and a
/// stranded drag takes every settings pane's wheel with it (see
/// `end_stranded_drag`). So the bar went dark under a hand that was still
/// resizing the pane, unpredictably, since it turns on crossing an edge.
#[test]
fn the_bar_being_dragged_stays_lit_when_the_pointer_leaves_the_window() {
    let style = theme::dock_style(&egui::Style::default(), 1.0);
    let mut h = DockHarness::new();
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let target = start_dragging_the_settings_boundary(&mut h, &mut state, &style);
    let out = h.frame(&mut state, vec![]);
    assert!(a_bar_is_lit(&out, &style), "the bar is not lit while it is being dragged");
    // The boundary itself, which is what the pointer is holding.
    let was = pane_rect(&state, panes::Tab::Tuning).left();
    // Back the way it came, which is the direction with room in it: the opening
    // leg spent 60 of the analyzer's 97 points over its floor, and a separator
    // stops at the floor of the pane it is drawn against rather than pushing
    // into the lattice behind it (see `fold::Points::slack`). Reversing is a
    // resize like any other — what this is about is the pointer being gone.
    h.frame(&mut state, vec![egui::Event::PointerGone]);
    h.frame(&mut state, vec![egui::Event::PointerMoved(target + egui::vec2(60.0, 0.0))]);
    let out = h.frame(&mut state, vec![]);
    let moved = pane_rect(&state, panes::Tab::Tuning).left() - was;
    assert!((moved - 60.0).abs() < 2.0, "the drag carried on for {moved:.0}pt of the 60 asked");
    assert!(a_bar_is_lit(&out, &style), "the bar went dark while the pane was still resizing");
}

/// A gesture the host takes focus from is over, and lets go of its bar.
///
/// The release goes to whoever has the window, so egui is left believing the
/// button is held forever — and anything that follows the pointer for as long as
/// it is held would follow it around the screen, resizing a pane nobody is
/// touching. Losing focus is the one unambiguous end of a gesture; a pointer that
/// has merely left the window is still dragging, which is the test above.
#[test]
fn a_drag_the_host_takes_focus_from_lets_go_of_its_bar() {
    let style = theme::dock_style(&egui::Style::default(), 1.0);
    let mut h = DockHarness::new();
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let target = start_dragging_the_settings_boundary(&mut h, &mut state, &style);
    h.frame(&mut state, vec![egui::Event::WindowFocused(false)]);
    let was = pane_rect(&state, panes::Tab::Tuning).left();
    h.frame(&mut state, vec![egui::Event::PointerMoved(target + egui::vec2(-60.0, 0.0))]);
    let out = h.frame(&mut state, vec![]);
    let moved = was - pane_rect(&state, panes::Tab::Tuning).left();
    assert!(moved.abs() < 1.0, "the pane followed a pointer the host had taken, by {moved:.0}pt");
    assert!(!a_bar_is_lit(&out, &style), "the bar stayed lit after the gesture was lost");
}

/// A window dragged narrow enough to squeeze a pane past its floor does NOT
/// re-dial the layout: the floor holds a drag, and a resize is not one.
///
/// The distinction is what keeps the floor from eating the layout. A resize moves
/// no fraction — that is what "the layout is the fractions" means — so the widths
/// it derives can dip under the floor and come back out of it, and the proportions
/// the user dialled are still there when the window returns.
#[test]
fn a_window_narrow_enough_to_squeeze_a_pane_does_not_re_dial_the_layout() {
    let mut h = DockHarness::new();
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    h.settle(&mut state);
    let panes = [panes::Tab::Lattice, panes::Tab::Spectral, panes::Tab::Tuning];
    let before = panes.map(|tab| pane_width(&state, tab));

    // Narrow enough that the analyzer cannot have its floor, and back again.
    let wide = h.screen.width();
    h.screen.max.x = h.screen.min.x + 420.0;
    h.settle(&mut state);
    let squeezed = pane_width(&state, panes::Tab::Spectral);
    assert!(squeezed < 102.0, "the point of the test is a pane under its floor, not {squeezed}");
    h.screen.max.x = h.screen.min.x + wide;
    h.settle(&mut state);

    for (tab, was) in panes.iter().zip(before) {
        let now = pane_width(&state, *tab);
        assert!((now - was).abs() < 1.5, "{tab:?} came back {now} wide, from {was}");
    }
}

/// Frameless mode hides every tab bar, so a folded pane there is squeezed to
/// NOTHING rather than to a rail — no rail, no name, no arrow. The separator it
/// left behind is still on screen, and still has a pane on either side of it, so
/// dragging it still resizes those two.
#[test]
fn a_pinned_separator_resizes_in_frameless_mode_too() {
    for delta in [40.0f32, -40.0] {
        let mut h = DockHarness::new();
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        h.settle(&mut state);
        let _ = h.collapse_click(&mut state, panes::Tab::Spectral);
        let _ = h.settle_folds(&mut state);
        state.view.frameless = true;
        let _ = h.settle_folds(&mut state);

        let gone = pane_rect(&state, panes::Tab::Spectral);
        assert!(gone.width() < 1.0, "a fold with no tab bar to leave is {} wide", gone.width());
        let before =
            (pane_width(&state, panes::Tab::Lattice), pane_width(&state, panes::Tab::Tuning));
        let at = egui::pos2(gone.right() + 2.0, gone.center().y);
        h.frame(&mut state, vec![egui::Event::PointerMoved(at)]);
        h.frame(&mut state, vec![egui::Event::PointerMoved(at), press(at, true)]);
        let target = at + egui::vec2(delta, 0.0);
        h.frame(&mut state, vec![egui::Event::PointerMoved(target)]);
        h.frame(&mut state, vec![press(target, false)]);
        let _ = h.settle_folds(&mut state);

        let after =
            (pane_width(&state, panes::Tab::Lattice), pane_width(&state, panes::Tab::Tuning));
        assert!(
            (after.0 - (before.0 + delta)).abs() < 1.5,
            "{delta:+}: the lattice should have taken the drag, and moved {}",
            after.0 - before.0,
        );
        assert!(
            (after.1 - (before.1 - delta)).abs() < 1.5,
            "{delta:+}: the settings column should have paid it, and moved {}",
            after.1 - before.1,
        );
        assert!(
            collapsed(&state, panes::Tab::Spectral),
            "{delta:+}: and the analyzer should still be folded — this was a resize",
        );
    }
}

/// The same, with the two collapsed panes SIBLINGS: their own split is collapsed
/// too, so one fold renders as two rails and the separator between them is inside
/// it. It has a rail on both sides and used to be inert, which from the outside
/// is a resize handle between two open panes that does nothing.
#[test]
fn the_separator_inside_a_folded_pair_resizes_the_open_panes_around_it() {
    let mut h = DockHarness::new();
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    // Lattice | (Spectral Tuning) | Notes, the middle two siblings.
    let mut dock = egui_dock::DockState::new(vec![panes::Tab::Lattice]);
    {
        let surface = dock.main_surface_mut();
        let [_, right] =
            surface.split_right(egui_dock::NodeIndex::root(), 0.4, vec![panes::Tab::Spectral]);
        let [pair, _] = surface.split_right(right, 0.5, vec![panes::Tab::Notes]);
        surface.split_right(pair, 0.5, vec![panes::Tab::Tuning]);
    }
    state.dock = dock;
    h.settle(&mut state);
    let _ = h.collapse_click(&mut state, panes::Tab::Spectral);
    let _ = h.collapse_click(&mut state, panes::Tab::Tuning);
    let _ = h.settle_folds(&mut state);

    let (first, last) = (panes::Tab::Lattice, panes::Tab::Notes);
    let before = (pane_width(&state, first), pane_width(&state, last));
    // Between the two rails, which is one fold's inside rather than its edge.
    let rails = (pane_rect(&state, panes::Tab::Spectral), pane_rect(&state, panes::Tab::Tuning));
    let at = egui::pos2((rails.0.right() + rails.1.left()) * 0.5, rails.0.center().y);
    h.frame(&mut state, vec![egui::Event::PointerMoved(at)]);
    h.frame(&mut state, vec![egui::Event::PointerMoved(at), press(at, true)]);
    let target = at - egui::vec2(50.0, 0.0);
    h.frame(&mut state, vec![egui::Event::PointerMoved(target)]);
    h.frame(&mut state, vec![press(target, false)]);
    let _ = h.settle_folds(&mut state);

    let after = (pane_width(&state, first), pane_width(&state, last));
    assert!(
        (after.0 - (before.0 - 50.0)).abs() < 1.5,
        "the lattice should have given up the 50 the drag took, and moved {}",
        after.0 - before.0,
    );
    assert!(
        (after.1 - (before.1 + 50.0)).abs() < 1.5,
        "and the pane on the other side of the rails should have taken it, and moved {}",
        after.1 - before.1,
    );
}

/// Where a fold holds the whole of one side of the window there is no boundary to
/// pass a drag out to — one open pane, one rail, and the window's own edge — so
/// the separator beside the rail has nothing it could resize, and neither a click
/// nor a drag on it does anything at all. The arrow on the rail is what brings
/// the pane back.
///
/// It could instead be that pane's own handle, unfolding it at the width it is
/// dragged to. What that costs is a pane the drag can leave narrower than
/// `separator.extra`, which is the floor egui_dock clamps every separator's
/// fraction to on every frame: under it the fraction saturates, `fold::drags`
/// reads a saturated fraction as that clamp rather than as a gesture, and the
/// pane comes back with a separator that lights up under the pointer and moves
/// nothing in either direction.
#[test]
fn the_handle_beside_an_outermost_rail_does_nothing() {
    let mut h = DockHarness::new();
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    h.settle(&mut state);
    // The Notes/Console leaf opens folded, so folding the settings leaf too
    // folds the whole column sideways into one rail down the right edge — and
    // leaves that fold with nothing outward to pass a drag to.
    let _ = h.collapse_click(&mut state, panes::Tab::Tuning);
    let _ = h.settle_folds(&mut state);
    let column = state
        .dock
        .find_tab(&panes::Tab::Tuning)
        .and_then(|path| path.node.parent())
        .expect("the settings leaf shares a column with the readouts");
    let main = egui_dock::SurfaceIndex::main();
    let rail = state.dock[main][column].rect().expect("the rail is laid out");
    let separator = theme::dock_style(&egui::Style::default(), 1.0).separator.width;
    let at = egui::pos2(rail.left() - separator * 0.5, rail.top() + 40.0);
    let lattice = pane_width(&state, panes::Tab::Lattice);

    // A click.
    h.frame(&mut state, vec![egui::Event::PointerMoved(at)]);
    h.frame(&mut state, vec![egui::Event::PointerMoved(at), press(at, true)]);
    h.frame(&mut state, vec![press(at, false)]);
    let _ = h.settle_folds(&mut state);
    assert!(
        collapsed(&state, panes::Tab::Tuning),
        "a click on the handle should have left the rail alone",
    );

    // And a drag well out into the open pane beside it.
    h.frame(&mut state, vec![egui::Event::PointerMoved(at), press(at, true)]);
    let target = egui::pos2(at.x - 260.0, at.y);
    h.frame(&mut state, vec![egui::Event::PointerMoved(target)]);
    h.frame(&mut state, vec![press(target, false)]);
    let _ = h.settle_folds(&mut state);
    assert!(
        collapsed(&state, panes::Tab::Tuning),
        "and so should a drag: the arrow is what opens a rail",
    );
    assert!(
        (pane_width(&state, panes::Tab::Lattice) - lattice).abs() < 1.0,
        "with no boundary to move, the pane beside the rail stays where it was: {} from {lattice}",
        pane_width(&state, panes::Tab::Lattice),
    );
}

/// Panes in a row, left to right, every split a horizontal one — so each of them
/// can be folded on its own account and each fold nests one split deeper than the
/// one to its left.
fn a_row_of(tabs: &[panes::Tab]) -> egui_dock::DockState<panes::Tab> {
    let mut dock = egui_dock::DockState::new(vec![tabs[0]]);
    let surface = dock.main_surface_mut();
    let mut node = egui_dock::NodeIndex::root();
    for (index, tab) in tabs.iter().enumerate().skip(1) {
        // An even share of what is left, so no pane starts so narrow that
        // egui_dock's own separator clamp has a say in where a drag lands.
        let share = 1.0 / (tabs.len() - index + 1) as f32;
        [_, node] = surface.split_right(node, share, vec![*tab]);
    }
    dock
}

/// "Reset layout" puts the pane WIDTHS back, not only the arrangement.
///
/// The dialled widths live in `fold::Dial::panes`, and the only thing that
/// re-seeds them from the dock is a change in the tree's node COUNT. Folding
/// never changes the tree's shape and a fresh `default_dock` has the count of
/// the dock it replaces, so a reset that does not say so leaves the widths
/// exactly where the user dragged them — the arrangement resets around them
/// and nothing moves.
#[test]
fn reset_layout_puts_the_dialled_pane_widths_back() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let style = theme::dock_style(&egui::Style::default(), 1.0);
    let mut h = DockHarness::new();
    h.settle_folds(&mut state);
    let fresh: Vec<f32> = LAID_OUT_TABS.iter().map(|tab| pane_width(&state, *tab)).collect();

    let target = start_dragging_the_settings_boundary(&mut h, &mut state, &style);
    h.frame(&mut state, vec![press(target, false)]);
    h.settle_folds(&mut state);
    let dragged: Vec<f32> = LAID_OUT_TABS.iter().map(|tab| pane_width(&state, *tab)).collect();
    // The analyzer, since that is the pane the settings boundary is drawn
    // against and a separator resizes what it divides (see `fold::Points::lean`).
    assert!(
        (dragged[1] - fresh[1]).abs() > 10.0,
        "the drag has to actually move a boundary for the reset to have something to undo: \
         {fresh:?} -> {dragged:?}"
    );

    state.reset_layout = true;
    h.settle_folds(&mut state);
    let reset: Vec<f32> = LAID_OUT_TABS.iter().map(|tab| pane_width(&state, *tab)).collect();
    for (index, tab) in LAID_OUT_TABS.iter().enumerate() {
        assert!(
            (reset[index] - fresh[index]).abs() < 1.0,
            "{tab:?} should be back at the width a fresh dock draws it at: \
             fresh {fresh:?}, dragged {dragged:?}, after the reset {reset:?}"
        );
    }
}

/// Loading a saved layout over a dialled one brings the SAVED widths, not the
/// ones the window is currently dragged to.
///
/// The plugin re-reads its chunk into the same long-lived `SharedState` every
/// time the editor opens, so this is the path a project reload takes — and the
/// saved dock has the node count of the one on screen, which is exactly what
/// the seed guard cannot see (see [`fold::Dial::forget`]).
#[test]
fn loading_a_saved_layout_brings_its_pane_widths_with_it() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let style = theme::dock_style(&egui::Style::default(), 1.0);
    let mut h = DockHarness::new();
    h.settle_folds(&mut state);
    let saved = state.save_persist();
    let fresh: Vec<f32> = LAID_OUT_TABS.iter().map(|tab| pane_width(&state, *tab)).collect();

    let target = start_dragging_the_settings_boundary(&mut h, &mut state, &style);
    h.frame(&mut state, vec![press(target, false)]);
    h.settle_folds(&mut state);
    let dragged: Vec<f32> = LAID_OUT_TABS.iter().map(|tab| pane_width(&state, *tab)).collect();
    // The analyzer, since that is the pane the settings boundary is drawn
    // against and a separator resizes what it divides (see `fold::Points::lean`).
    assert!((dragged[1] - fresh[1]).abs() > 10.0, "the drag has to move a boundary: {dragged:?}");

    state.load_persist(&saved);
    h.settle_folds(&mut state);
    let loaded: Vec<f32> = LAID_OUT_TABS.iter().map(|tab| pane_width(&state, *tab)).collect();
    for (index, tab) in LAID_OUT_TABS.iter().enumerate() {
        assert!(
            (loaded[index] - fresh[index]).abs() < 1.0,
            "{tab:?} should be back at the width it was saved at: \
             saved {fresh:?}, dragged {dragged:?}, loaded {loaded:?}"
        );
    }
}

/// The panes the default dock lays out side by side, left to right.
const LAID_OUT_TABS: [panes::Tab; 3] =
    [panes::Tab::Lattice, panes::Tab::Spectral, panes::Tab::Tuning];

fn pane_rect(state: &SharedState, tab: panes::Tab) -> egui::Rect {
    let path = state.dock.find_tab(&tab).expect("the tab is docked");
    state.dock[path.surface][path.node].rect().expect("the pane is laid out")
}

fn pane_width(state: &SharedState, tab: panes::Tab) -> f32 {
    pane_rect(state, tab).width()
}

/// The pane names painted up a rail this frame, top to bottom.
///
/// `fold::paint` sets them a quarter turn anticlockwise, which nothing else in
/// the dock does — so the angle is what tells a rail's name from a tab title,
/// and the rail's own rectangle is what keeps another pane's rotated text out.
///
/// Each comes with the y its text STARTS at — the top of the drawn word. A
/// name is anchored at its far end and drawn upwards, so that is the anchor
/// less the galley's length.
fn rail_labels(output: &egui::FullOutput, rail: egui::Rect) -> Vec<(String, f32)> {
    fn walk(shape: &egui::Shape, rail: egui::Rect, found: &mut Vec<(f32, String)>) {
        match shape {
            egui::Shape::Text(text) if text.angle != 0.0 && rail.contains(text.pos) => {
                found.push((text.pos.y - text.galley.size().x, text.galley.text().to_owned()));
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, rail, found)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &output.shapes {
        walk(&clipped.shape, rail, &mut found);
    }
    found.sort_by(|(a, _), (b, _)| a.total_cmp(b));
    found.into_iter().map(|(top, name)| (name, top)).collect()
}

/// The rail a folded subtree left behind: the leaves' rectangles, which are
/// the rail's own once they have nothing else in them.
fn rail_rect(state: &SharedState, tabs: &[panes::Tab]) -> egui::Rect {
    tabs.iter().fold(egui::Rect::NOTHING, |rail, tab| {
        let path = state.dock.find_tab(tab).expect("tab is in the dock");
        rail.union(state.dock[path.surface][path.node].rect().expect("the leaf is laid out"))
    })
}

/// A folded column names every pane in it, not just the one at the bottom.
///
/// egui_dock stacks a collapsed leaf's tab bar at the top of the column and
/// hands everything below it to whichever leaf is LAST, so the rail carried a
/// single name — "Notes", the pane at the bottom of the settings column and
/// the one that says least about what folded. The names now divide the rail
/// by the fractions the column is dialled at, so both of them are on it.
#[test]
fn a_folded_column_names_every_pane_in_it() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.min_window_width = 400.0;
    let mut h = DockHarness::new();
    h.settle(&mut state);
    // Notes/Console is folded in the default layout, so collapsing the
    // settings leaf collapses the column itself and the whole of it folds
    // sideways to one rail.
    let output = h.collapse_click(&mut state, panes::Tab::Tuning);
    let rail = rail_rect(&state, &[panes::Tab::Tuning, panes::Tab::Notes]);
    assert!(rail.width() < 40.0, "the column should have folded to a rail ({rail:?})");
    let labels = rail_labels(&output, rail);
    assert_eq!(
        labels.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(),
        ["Tuning", "Notes"],
        "both panes in the folded column should be named, in the order they are stacked",
    );
    // Each in ITS share of the rail, under its own arrow: the shares are the
    // column's own fractions (the log leaf is dialled at 0.45 of it), and a
    // name hangs one tab bar plus a little padding below the top of its share,
    // which is the arrow that brings that pane back.
    let boundary = rail.top() + rail.height() * 0.55;
    let under = |top: f32| top..top + crate::theme::TAB_BAR_HEIGHT + 20.0;
    assert!(
        under(rail.top()).contains(&labels[0].1),
        "the upper pane's name should sit under the arrow at the top of the rail: {labels:?}",
    );
    assert!(
        under(boundary).contains(&labels[1].1),
        "the lower pane's name should sit under its own arrow at {boundary}: {labels:?}",
    );
}

/// Two panes folded side by side name themselves under their own arrows.
///
/// A folded pair is two rails, and their collapse arrows end up next to each
/// other at the top of the window pointing the SAME way — both panes come back
/// into the space the pair gave up, so the direction cannot separate them.
/// That leaves the name, and a name floating at the middle of its rail is most
/// of a window away from the arrow it belongs to. Under the arrow the two read
/// as a caption each.
#[test]
fn a_folded_pair_names_each_rail_under_its_own_arrow() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.min_window_width = 400.0;
    let mut h = DockHarness::new();
    h.settle(&mut state);
    // The picture pair: collapsing both leaves the split itself collapsed, so
    // the whole subtree folds as one and comes out two rails wide.
    h.collapse_click(&mut state, panes::Tab::Lattice);
    let output = h.collapse_click(&mut state, panes::Tab::Spectral);

    for tab in [panes::Tab::Lattice, panes::Tab::Spectral] {
        let rail = rail_rect(&state, &[tab]);
        assert!(rail.width() < 40.0, "{tab:?} should have folded to a rail ({rail:?})");
        let labels = rail_labels(&output, rail);
        let (name, top) = labels.first().expect("a rail says which pane it is");
        assert_eq!(name, crate::panes::tab_title(&tab), "each rail names its own pane");
        // The arrow is the leaf's tab bar, at the top of the rail. Clear of
        // it, and within a word's length of it — not adrift at mid-window,
        // which for this dock would be past 380.
        let arrow = rail.top() + crate::theme::TAB_BAR_HEIGHT;
        assert!(
            (arrow..arrow + 20.0).contains(top),
            "{name} sits at {top}, and its arrow ends at {arrow}",
        );
    }
}

/// A click on the arrow at the top of a folded column's LOWER share opens the
/// pane that share belongs to.
///
/// egui_dock puts a collapsed leaf's arrow at the top of its own rectangle,
/// and down a folded column those rectangles start one after another — so its
/// arrows stack in the first inches of the rail, out of reach of the panes
/// they name. `fold` places them on the shares instead and takes the clicks
/// itself, which is the half of this that cannot be done in paint alone.
#[test]
fn a_folded_columns_lower_arrow_opens_its_own_pane() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.min_window_width = 400.0;
    let mut h = DockHarness::new();
    h.settle(&mut state);
    h.collapse_click(&mut state, panes::Tab::Tuning);
    let rail = rail_rect(&state, &[panes::Tab::Tuning, panes::Tab::Notes]);
    assert!(collapsed(&state, panes::Tab::Tuning) && collapsed(&state, panes::Tab::Notes));

    // The lower share's own arrow, at 0.55 down the rail.
    let at = egui::pos2(
        rail.left() + 12.0,
        rail.top() + rail.height() * 0.55 + crate::theme::TAB_BAR_HEIGHT * 0.5,
    );
    h.frame(&mut state, vec![egui::Event::PointerMoved(at)]);
    h.frame(&mut state, vec![egui::Event::PointerMoved(at), press(at, true)]);
    h.frame(&mut state, vec![press(at, false)]);
    h.settle_folds(&mut state);

    assert!(!collapsed(&state, panes::Tab::Notes), "the lower arrow opens the lower pane");
    assert!(
        collapsed(&state, panes::Tab::Tuning),
        "and only that one — the settings leaf keeps its own arrow",
    );
    // The column is a column again, and the leaf still folded is one tab bar
    // tall. That last one is where a botched collapsed-leaf count shows up:
    // not as a pane that fails to open, but as a bar drawn some multiple of a
    // tab bar thick afterwards (see `fold::uncollapse`).
    let path = state.dock.find_tab(&panes::Tab::Tuning).expect("tab is in the dock");
    let folded = state.dock[path.surface][path.node].rect().expect("laid out");
    assert!(folded.width() > 100.0, "the rail should be a column again: {folded:?}");
    assert!(
        (folded.height() - crate::theme::TAB_BAR_HEIGHT).abs() < 4.0,
        "the settings leaf should be one tab bar tall, not {}",
        folded.height(),
    );
}

/// The arrow egui_dock left stacked at the top of the rail no longer opens
/// anything: it sits inside the share above it now, under another pane's name,
/// and a click there would open a pane the rail says nothing about.
#[test]
fn a_folded_columns_stacked_arrow_is_inert() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.min_window_width = 400.0;
    let mut h = DockHarness::new();
    h.settle(&mut state);
    h.collapse_click(&mut state, panes::Tab::Tuning);
    let rail = rail_rect(&state, &[panes::Tab::Tuning, panes::Tab::Notes]);

    // Where egui_dock puts the log leaf's button: directly under the settings
    // leaf's, one tab bar down from the top of the rail.
    let path = state.dock.find_tab(&panes::Tab::Notes).expect("tab is in the dock");
    let stacked = state.dock[path.surface][path.node].rect().expect("laid out");
    let at = stacked.left_top() + egui::vec2(12.0, crate::theme::TAB_BAR_HEIGHT * 0.5);
    assert!(
        at.y < rail.top() + rail.height() * 0.5,
        "the stacked arrow should be near the top of the rail, at {at:?}",
    );
    h.frame(&mut state, vec![egui::Event::PointerMoved(at)]);
    h.frame(&mut state, vec![egui::Event::PointerMoved(at), press(at, true)]);
    h.frame(&mut state, vec![press(at, false)]);
    h.settle_folds(&mut state);

    assert!(
        collapsed(&state, panes::Tab::Notes),
        "the log pane should not open from a button that is no longer drawn",
    );
}

/// A whole pointer gesture on a separator: press where it is drawn, travel `dx`
/// in steps, release. Stepped rather than jumped, because a drag is followed by
/// the pointer frame to frame once the boundary is held (see `fold::Grip`).
fn drag_separator(h: &mut DockHarness, state: &mut SharedState, from: egui::Pos2, dx: f32) {
    h.frame(state, vec![egui::Event::PointerMoved(from)]);
    h.frame(state, vec![egui::Event::PointerMoved(from), press(from, true)]);
    for step in 1..=10 {
        let at = from + egui::vec2(dx * step as f32 / 10.0, 0.0);
        h.frame(state, vec![egui::Event::PointerMoved(at)]);
    }
    let end = from + egui::vec2(dx, 0.0);
    h.frame(state, vec![press(end, false)]);
    h.settle_folds(state);
}

/// The separator drawn against a pane's left edge, where the pointer grabs it.
fn handle_left_of(state: &SharedState, tab: panes::Tab, style: &egui_dock::Style) -> egui::Pos2 {
    let rect = pane_rect(state, tab);
    egui::pos2(rect.left() - style.separator.width * 0.5, rect.center().y)
}

/// With the analyzer squeezed to its floor, its RIGHT-hand separator stops
/// rather than moving the lattice — and its own arrow of travel, outward, still
/// works.
///
/// Reported as the right handle grabbing the left one, which is what it looks
/// like: pushing the drag through the floor into the lattice leaves the analyzer
/// sliding sideways at a fixed width, so the boundary that visibly moves against
/// the panes is the one on the analyzer's OTHER side. Both handles do move, but
/// the pane the gesture was aimed at is the one that does not.
///
/// Driven through the real dock rather than `fold::tests`, because it is a
/// claim about what a pointer on a separator does, and egui_dock's own drag and
/// its per-frame re-clamp are both between the two.
#[test]
fn a_squeezed_panes_far_separator_stops_rather_than_moving_the_pane_behind_it() {
    let style = theme::dock_style(&egui::Style::default(), 1.0);
    // Inward the analyzer has nothing left to give and the boundary holds;
    // outward it is an ordinary resize, and travels eight of the gesture's ten
    // steps: the press frame carries no travel of its own, and the fraction a
    // drag writes is read on the frame after the one it was written in.
    for (what, dx, want) in [("inward", -60.0f32, 0.0f32), ("outward", 60.0, 48.0)] {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        let mut h = DockHarness::new();
        h.settle_folds(&mut state);
        // Squeeze the analyzer to its floor with its own LEFT handle.
        let at = handle_left_of(&state, panes::Tab::Spectral, &style);
        drag_separator(&mut h, &mut state, at, 200.0);
        let floor = style.separator.extra - style.separator.width * 0.5;
        let (lattice, analyzer) =
            (pane_width(&state, panes::Tab::Lattice), pane_width(&state, panes::Tab::Spectral));
        assert!((analyzer - floor).abs() < 1.0, "the analyzer is not on its floor: {analyzer}");

        let grabbed = handle_left_of(&state, panes::Tab::Tuning, &style);
        let boundary = pane_rect(&state, panes::Tab::Tuning).left();
        drag_separator(&mut h, &mut state, grabbed, dx);
        let moved = pane_rect(&state, panes::Tab::Tuning).left() - boundary;
        assert!(
            (moved - want).abs() < 1.0,
            "dragged {what}, the boundary went {moved:.1}pt where it should have gone {want}",
        );
        assert!(
            (pane_width(&state, panes::Tab::Lattice) - lattice).abs() < 1.0,
            "dragged {what}, the lattice is not what this separator divides, and moved {:.1}pt",
            pane_width(&state, panes::Tab::Lattice) - lattice,
        );
    }
}
