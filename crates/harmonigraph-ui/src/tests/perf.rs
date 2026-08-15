//! The performance overlay: where it opens, that it is dragged from there,
//! that nothing else moves it, and that it ships off.

use crate::*;
use super::harness::*;

/// The HUD's backing plate, which is its actual extent — the rows inside it
/// are left-aligned, so no single string reveals where the box sits.
///
/// Found by its painted plate rather than by `Memory::area_rect` for a second
/// reason: what is worth asserting is where the numbers LAND, and an Area's
/// rect is a frame behind its contents on the pass that sizes it.
fn hud_of(output: &egui::FullOutput) -> egui::Rect {
    let plate = egui::Color32::from_black_alpha(0xC0);
    output
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) if rect.fill == plate => Some(rect.rect),
            _ => None,
        })
        .expect("the overlay should paint its backing plate")
}

/// A press-move-release on the HUD, from its middle by `by`, and the plate as
/// it stands after the release.
fn drag_hud(h: &mut DockHarness, state: &mut SharedState, by: egui::Vec2) -> egui::Rect {
    let grab = hud_of(&h.frame(state, vec![])).center();
    let to = grab + by;
    h.frame(state, vec![egui::Event::PointerMoved(grab)]);
    h.frame(state, vec![egui::Event::PointerMoved(grab), press(grab, true)]);
    h.frame(state, vec![egui::Event::PointerMoved(to)]);
    h.frame(state, vec![egui::Event::PointerMoved(to), press(to, false)]);
    hud_of(&h.frame(state, vec![]))
}

/// An overlay nobody has dragged opens in the editor's bottom-right corner —
/// the one placement anything but a drag decides.
///
/// Read off the EDITOR, and off nothing else. What pane is where, which leaf
/// is folded, where the tab bars run: none of it is consulted, and that is the
/// point rather than an omission. The corner is where the HUD opens, not where
/// it belongs, and a drag is what settles the question for good.
#[test]
fn the_perf_overlay_opens_in_the_editors_corner() {
    let mut state = fresh();
    // Turned on by hand: the overlay ships off, and what is under test is
    // where it lands once asked for, not whether anything asks.
    state.view.show_perf = true;
    let mut h = DockHarness::new();
    let screen = h.screen;
    // An Area's opening pass sizes it and paints nothing, so the HUD is read
    // from the pass after.
    h.frame(&mut state, vec![]);
    let output = h.frame(&mut state, vec![]);

    assert!(
        output.shapes.iter().any(|clipped| matches!(
            &clipped.shape,
            egui::Shape::Text(text) if text.galley.text().contains("fps")
        )),
        "the overlay should be drawn once show_perf is set",
    );
    let hud = hud_of(&output);
    assert!(screen.contains_rect(hud), "the HUD should sit inside the editor: {hud:?}");
    assert!(
        (hud.right() - (screen.right() - 8.0)).abs() < 1.0,
        "the HUD should hug the editor's RIGHT edge: {hud:?} in {screen:?}",
    );
    assert!(
        (hud.bottom() - (screen.bottom() - 8.0)).abs() < 1.0,
        "the HUD should hug the editor's BOTTOM edge: {hud:?} in {screen:?}",
    );
    // Nothing has placed it, which is what makes the corner a default: the
    // position is written by the drag and by nothing else.
    assert!(state.perf_pos.is_none(), "opening the HUD must not place it");

    // The build tag, which is why the HUD is worth looking at before any of
    // its numbers: Bitwig loads ONE bundle and every session builds into its
    // own worktree, so "am I even looking at the build I just loaded?" has a
    // wrong answer available. Asserted as painted TEXT, because a tag that is
    // computed and not drawn would verify nothing.
    //
    // Wrapping means the tag can span two galleys, so this looks for the
    // branch name rather than the whole line.
    let branch =
        harmonigraph_perf::BUILD_TAG.split(" @").next().unwrap_or(harmonigraph_perf::BUILD_TAG);
    assert!(
        output.shapes.iter().any(|clipped| matches!(
            &clipped.shape,
            egui::Shape::Text(text) if text.galley.text().contains(branch)
        )),
        "the overlay should name the build it is ({}), so a reload can be checked",
        harmonigraph_perf::BUILD_TAG,
    );
}

/// The corner holds on the frame the HUD's own rows change size.
///
/// An undragged HUD is placed from two readings — the editor rect and the
/// measured rows — and the frame the second one changes is the frame they can
/// disagree. `egui::Area` measures its containment against the size it held
/// LAST frame, so a HUD that just lost twenty rows was shoved a block of rows
/// clear of the corner for one frame and snapped back on the next. Unticking
/// Frame breakdown is exactly that input, and it is the control someone
/// unticks while watching the HUD.
#[test]
fn the_perf_overlay_holds_its_corner_as_its_rows_shrink() {
    let mut state = fresh();
    state.view.show_perf = true;
    // The breakdown open, which is the tall HUD: two dozen rows against four.
    state.view.show_perf_detail = true;
    let mut h = DockHarness::new();
    h.settle(&mut state);
    let screen = h.screen;

    let deep = hud_of(&h.frame(&mut state, vec![]));
    assert!(
        (deep.bottom() - (screen.bottom() - 8.0)).abs() < 1.0,
        "the breakdown should open in the corner too: {deep:?}",
    );

    // The VERY next frame after the rows go: no settling, because one frame
    // in the wrong place is the whole of what this is about.
    state.view.show_perf_detail = false;
    let shallow = hud_of(&h.frame(&mut state, vec![]));
    assert!(
        shallow.height() < deep.height() - 100.0,
        "the breakdown should have closed: {deep:?} then {shallow:?}",
    );
    assert!(
        (shallow.bottom() - (screen.bottom() - 8.0)).abs() < 1.0,
        "the HUD left the corner as its rows shrank: {shallow:?} in {screen:?}",
    );
    assert!(
        (shallow.right() - (screen.right() - 8.0)).abs() < 1.0,
        "the HUD left the corner as its rows shrank: {shallow:?} in {screen:?}",
    );
}

/// The HUD is dragged, and it is dropped where the pointer left it.
///
/// Through the REAL dock, so the drag crosses every layer between the mouse
/// and the overlay: the picture pane under it senses drags of its own (a drag
/// on the lattice orbits the camera), and the HUD sitting on a foreground
/// layer is what decides which of the two the pointer is talking to.
#[test]
fn the_perf_overlay_goes_where_it_is_dragged() {
    let mut state = fresh();
    state.view.show_perf = true;
    let mut h = DockHarness::new();
    h.settle(&mut state);

    let before = hud_of(&h.frame(&mut state, vec![]));
    // Up and to the left, which is into the editor from the corner the HUD
    // opens in — a drag the containment cannot be mistaken for.
    let by = egui::vec2(-260.0, -190.0);
    let after = drag_hud(&mut h, &mut state, by);

    assert!(
        (after.min - (before.min + by)).length() < 1.0,
        "the HUD should have moved by the drag: {before:?} by {by:?} left it at {after:?}",
    );
    // Within a point of where it is drawn rather than equal to it: what is
    // stored is where the pointer left the HUD, and what is painted is that
    // rounded to a whole pixel, which is egui's job and not this one's.
    let recorded = state.perf_pos.expect("the drop should be recorded");
    assert!(
        (recorded - after.min).length() < 1.0,
        "the drop should be recorded as the overlay's position: {recorded:?} drawn at {after:?}",
    );
    // Drawn where it was dropped on the frames after, rather than springing
    // back to the corner once the pointer lets go.
    let settled = hud_of(&h.frame(&mut state, vec![]));
    assert_eq!(settled, after, "the HUD should stay where it was dropped");
}

/// Once placed, the HUD is where the user put it, whatever the dock does.
///
/// This is the inverse of a rule the overlay used to carry: it hung off the
/// analyzer pane, fell back to the lattice when that pane went off screen, and
/// off the editor when neither was up — so folding a leaf moved it. A dragged
/// HUD is furniture the user positioned, and a fold is not a request to move
/// it.
#[test]
fn folding_a_pane_does_not_move_the_perf_overlay() {
    let mut state = fresh();
    state.view.show_perf = true;
    let mut h = DockHarness::new();
    h.settle(&mut state);
    let placed = drag_hud(&mut h, &mut state, egui::vec2(-200.0, -120.0));

    // Both picture panes off screen in turn — the two folds that used to hand
    // the overlay from one pane to the next, and then to the window.
    for tab in [panes::Tab::Spectral, panes::Tab::Lattice] {
        let path = state.workspace.dock.find_tab(&tab).expect("the tab is docked");
        let egui_dock::Node::Leaf(leaf) = &mut state.workspace.dock[path.surface][path.node] else {
            panic!("{tab:?} should live in a leaf");
        };
        leaf.collapsed = true;
        h.settle_folds(&mut state);
        // Its POSITION, which is what a fold used to change. The plate's width
        // still follows the numbers inside it — a folded lattice draws no
        // nodes, and that row gets shorter — and it grows from the corner the
        // HUD was dropped by.
        assert_eq!(
            hud_of(&h.frame(&mut state, vec![])).min,
            placed.min,
            "collapsing {tab:?} moved the overlay",
        );
    }
}

/// A drag cannot push the HUD out of the editor, because the plate is the only
/// handle it has: dragged off the edge, or left where a smaller window no
/// longer reaches, there would be nothing left to grab it by.
#[test]
fn the_perf_overlay_cannot_be_dragged_out_of_the_editor() {
    let mut state = fresh();
    state.view.show_perf = true;
    let mut h = DockHarness::new();
    h.settle(&mut state);
    let screen = h.screen;

    // Far past the bottom-left corner, which is as far as a pointer inside the
    // window can throw it.
    let hud = drag_hud(&mut h, &mut state, egui::vec2(-2000.0, 2000.0));
    assert!(
        screen.contains_rect(hud),
        "the HUD was dragged out of reach: {hud:?} outside {screen:?}",
    );

    // ...and a window that shrinks under a HUD already at its edge brings it
    // back in rather than leaving it beyond the corner.
    h.screen.max -= egui::vec2(300.0, 200.0);
    h.frame(&mut state, vec![]);
    let hud = hud_of(&h.frame(&mut state, vec![]));
    assert!(
        h.screen.contains_rect(hud),
        "a smaller window stranded the HUD: {hud:?} outside {:?}",
        h.screen,
    );
}

/// A position that came out of a blob draws where it says, on a context that
/// has never seen the HUD.
///
/// The gap this closes is the one a person actually opens: reopening the
/// editor builds a BRAND NEW `egui::Context` (see `release_context_resources`)
/// while the state lives across it, so the Area has no memory of its size and
/// egui spends the first pass measuring. Every other test here reaches a
/// placed HUD through a drag, on a context that already knows it. Without
/// this, a `draw_overlay` that wrote back on a non-drag frame would corrupt
/// the saved position on every window open and the suite would stay green.
#[test]
fn a_saved_position_opens_the_hud_where_it_was_left() {
    let mut state = fresh();
    state.view.show_perf = true;
    // Nowhere near the corner it would open at by itself, so "honoured" and
    // "defaulted" cannot look alike.
    let left = egui::pos2(120.0, 210.0);
    state.perf_pos = Some(left);

    let mut h = DockHarness::new();
    h.frame(&mut state, vec![]);
    let hud = hud_of(&h.frame(&mut state, vec![]));
    assert!(
        (hud.min - left).length() < 1.0,
        "a saved position should be where the HUD opens: {hud:?} for {left:?}",
    );
    assert_eq!(state.perf_pos, Some(left), "drawing the HUD must not rewrite its position");
}

/// Switching the HUD off and on again leaves it where it was, and draws it on
/// the frame it comes back rather than costing another blank one.
///
/// Both hold because egui keeps a hidden Area's state, which is a fact about
/// egui rather than about this code — which is exactly why it is worth an
/// assertion here.
#[test]
fn the_perf_overlay_comes_back_where_it_was_switched_off() {
    let mut state = fresh();
    state.view.show_perf = true;
    let mut h = DockHarness::new();
    h.settle(&mut state);
    let placed = drag_hud(&mut h, &mut state, egui::vec2(-180.0, -140.0));

    state.view.show_perf = false;
    h.frame(&mut state, vec![]);
    state.view.show_perf = true;
    // ONE frame: a second would hide a fresh sizing pass behind it.
    let back = hud_of(&h.frame(&mut state, vec![]));
    assert_eq!(back, placed, "the HUD should come back where it was switched off");
}

/// Where the HUD was dragged to outlives the session, and a hand-edited
/// position that cannot be drawn is dropped rather than honoured.
#[test]
fn a_dragged_perf_overlay_is_persisted() {
    let mut state = fresh();
    state.view.show_perf = true;
    state.perf_pos = Some(egui::pos2(123.0, 456.0));

    let mut restored = fresh();
    assert!(restored.load_persist(&state.save_persist()));
    assert_eq!(
        restored.perf_pos,
        Some(egui::pos2(123.0, 456.0)),
        "a dragged overlay should open where it was left",
    );

    // NaN reaches the tessellator as geometry, and a blob is a file someone
    // can edit — see `load_persist`, which repairs the spiral framing beside
    // this for the same reason.
    let mut restored = fresh();
    let saved = state.save_persist();
    let edited = saved.replacen("x:123.0", "x:NaN", 1);
    assert_ne!(edited, saved, "the position must be in the blob to edit");
    assert!(restored.load_persist(&edited));
    assert_eq!(restored.perf_pos, None, "an undrawable position should be dropped");
}

/// The overlay ships OFF, on a fresh install and in a project saved before the
/// setting existed alike.
///
/// Two separate declarations decide this and they have to agree: the struct
/// default is what a fresh install reads, and `#[serde(default)]` is what a
/// blob missing the key reads. A fresh install exercises only the first, so a
/// disagreement stays invisible until someone opens an old project and finds a
/// HUD sitting over the picture. Both are asserted here for that reason.
#[test]
fn the_performance_overlay_ships_off() {
    let defaults = fresh();
    assert!(!defaults.view.show_perf, "a fresh install opens with the overlay off");

    // A blob from before the setting existed: the key cut out of a saved one.
    let mut state = fresh();
    state.view.show_perf = true;
    let saved = state.save_persist();
    let old = saved.replacen("show_perf:true,", "", 1);
    assert_ne!(old, saved, "the show_perf cut must land for this to test anything");

    let mut restored = fresh();
    restored.load_persist(&old);
    assert!(!restored.view.show_perf, "a pre-show_perf blob opens with the overlay off");

    // And a project that asked for it still gets it: the cut above is what
    // makes the blob old, not the value, so the round-trip has to still work.
    let mut kept = fresh();
    kept.load_persist(&saved);
    assert!(kept.view.show_perf, "a project that turned the overlay on keeps it");
}
