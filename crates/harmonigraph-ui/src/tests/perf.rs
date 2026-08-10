//! The performance overlay: where it hangs, and that it ships off.

use crate::*;
use harmonigraph_render::wgpu::TextureFormat;
use super::harness::*;

/// The performance overlay hangs off the analyzer pane; off the lattice when
/// that pane isn't on screen; off the editor, clear of the tab bar, when
/// neither is. All three land somewhere no tab bar's collapse arrow is.
#[test]
fn the_perf_overlay_follows_the_analyzer_pane() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    // Turned on by hand: the overlay ships off, and what is under test is where
    // it lands once asked for, not whether anything asks.
    state.view.show_perf = true;
    let backend = RecordingBackend::default();
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 800.0));
    let mut t = 0.0;
    let mut frame = |state: &mut SharedState| {
        t += 1.0 / 60.0;
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(t),
            ..Default::default()
        };
        ctx.run_ui(raw, |ui| root_ui(ui, state, &backend, t))
    };
    // A frame first: the dock only knows where its panes are once it has laid
    // them out, and before that the overlay has nothing to hang off.
    frame(&mut state);
    let output = frame(&mut state);

    let area = perf_overlay_area(&state, screen, 1.0);
    assert_ne!(area, screen, "the overlay should have found the analyzer pane");

    // ...and the HUD really lands in that pane's top-right corner.
    //
    // Found by its painted text rather than by `Memory::area_rect`: the HUD is
    // not an Area. As one, every label inside it registers a widget rect that
    // takes the pointer from whatever is underneath — a dead zone the size of
    // the readout. It is painted straight onto a foreground layer, so there is
    // no area to look up, and the thing worth asserting was never the Area
    // anyway: it is where the numbers land.
    assert!(
        output.shapes.iter().any(|clipped| matches!(
            &clipped.shape,
            egui::Shape::Text(text) if text.galley.text().contains("fps")
        )),
        "the overlay should be drawn once show_perf is set",
    );
    // The backing plate, which is the HUD's actual extent — the rows inside it
    // are left-aligned, so no single string reveals where the box sits.
    let hud = hud_of(&output);
    assert!(area.contains_rect(hud), "the HUD should sit inside the analyzer pane: {hud:?}");
    assert!(
        (hud.right() - (area.right() - 8.0)).abs() < 1.0,
        "the HUD should hug the pane's RIGHT edge: {hud:?} in {area:?}",
    );
    assert!(
        (hud.right() - area.right()).abs() < 12.0 && (hud.top() - area.top()).abs() < 12.0,
        "the HUD should hug the pane's top-RIGHT corner: {hud:?} in {area:?}",
    );
    // The build tag, which is why the HUD is worth looking at before any of
    // its numbers: Bitwig loads ONE bundle and every session builds into its
    // own worktree, so "am I even looking at the build I just loaded?" has a
    // wrong answer available. Asserted as painted TEXT, because a tag that is
    // computed and not drawn would verify nothing.
    //
    // Wrapping means the tag can span two galleys, so this looks for the
    // branch name rather than the whole line.
    let branch = perf::BUILD_TAG.split(" @").next().unwrap_or(perf::BUILD_TAG);
    assert!(
        output.shapes.iter().any(|clipped| matches!(
            &clipped.shape,
            egui::Shape::Text(text) if text.galley.text().contains(branch)
        )),
        "the overlay should name the build it is ({}), so a reload can be checked",
        perf::BUILD_TAG,
    );
    // ...and naming it must not have pushed the HUD out of its pane. The tag
    // is a branch name, so it is arbitrarily long; `draw_overlay` wraps it to
    // the width the numbers already need. Without that, a long enough branch
    // silently widens the HUD past the pane — which the assertion above on
    // `contains_rect` catches, but only on a branch that happens to be long.
    assert!(
        hud.width() < area.width(),
        "the build tag must wrap, not widen the HUD: {hud:?} in {area:?}",
    );

    assert!(screen.contains_rect(area), "the analyzer pane is inside the editor");
    // Right of the lattice and left of the settings column: the Spectral pane
    // as `default_dock` places it.
    assert!(area.left() > screen.left(), "the analyzer pane is not the left edge");
    assert!(area.right() < screen.right(), "the settings column is right of it");

    // Its leaf holds Spectral alone, so collapsing it is what takes it off
    // screen; the overlay then falls back to the OTHER picture pane.
    let path = state.workspace.dock.find_tab(&panes::Tab::Spectral).expect("Spectral is docked");
    let egui_dock::Node::Leaf(leaf) = &mut state.workspace.dock[path.surface][path.node] else {
        panic!("Spectral should live in a leaf");
    };
    leaf.collapsed = true;
    frame(&mut state);
    let output = frame(&mut state);
    let lattice = state.workspace.dock.find_tab(&panes::Tab::Lattice).expect("Lattice is docked");
    let egui_dock::Node::Leaf(lattice) = &state.workspace.dock[lattice.surface][lattice.node] else {
        panic!("Lattice should live in a leaf");
    };
    assert_eq!(
        perf_overlay_area(&state, screen, 1.0),
        lattice.viewport,
        "a collapsed analyzer should hand the overlay to the lattice, not to the window",
    );

    // ...and the point of that, which is what the fallback is FOR: the HUD is
    // painted on a foreground layer over the whole dock, so hanging it off the
    // window puts it on the chrome along the top — the settings column's tab
    // bar, and the collapse arrow at the left of every bar, which is the
    // control that brings a folded pane back. A tab body starts below its own
    // bar, so landing in one is what keeps it clear of all of them.
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
    fn clear_of_every_tab_bar(state: &SharedState, hud: egui::Rect, what: &str) {
        for node in state.workspace.dock.main_surface().iter() {
            let egui_dock::Node::Leaf(leaf) = node else {
                continue;
            };
            let mut bar = leaf.rect;
            bar.max.y = bar.min.y + crate::theme::TAB_BAR_HEIGHT;
            assert!(
                !hud.intersects(bar),
                "{what}: the HUD covers a tab bar and its collapse arrow: {hud:?} over {bar:?}",
            );
        }
    }
    clear_of_every_tab_bar(&state, hud_of(&output), "on the lattice");

    // Fold the lattice too and there is no picture left to hang off, which is
    // the last resort. It is the only branch that does arithmetic — the editor
    // rect pushed down past the tab bar — and the arithmetic is the whole of
    // what keeps the HUD off the collapse arrows in the one state where those
    // arrows are the only way back.
    let path = state.workspace.dock.find_tab(&panes::Tab::Lattice).expect("Lattice is docked");
    let egui_dock::Node::Leaf(leaf) = &mut state.workspace.dock[path.surface][path.node] else {
        panic!("Lattice should live in a leaf");
    };
    leaf.collapsed = true;
    frame(&mut state);
    let output = frame(&mut state);
    let area = perf_overlay_area(&state, screen, 1.0);
    assert_eq!(
        area.min.y,
        screen.min.y + crate::theme::TAB_BAR_HEIGHT,
        "with neither picture on screen the overlay should clear the tab bar: {area:?}",
    );
    clear_of_every_tab_bar(&state, hud_of(&output), "with both pictures folded");
}

/// Landing in the analyzer's body is only half of staying out of the way: the
/// HUD is painted on a foreground layer whose clip is the whole screen, so a
/// pane too narrow to hold it does not crop it — it spills across the separator
/// and over the settings column, and over whatever collapse arrow is there.
///
/// The analyzer is the NARROWEST pane in `default_dock` (0.2016 of the window),
/// so it is the first to run out of room, and a sideways fold can drive the
/// window to its floor without the user ever dragging it there.
#[test]
fn the_perf_overlay_stays_inside_its_pane_at_the_narrowest_window() {
    // The plugin's own minimum editor width (`MIN_SIZE` in the plugin crate's
    // editor), which is a window the shell will actually hand the UI.
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 800.0));
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    // Turned on by hand: the overlay ships off, and what is under test is
    // whether it stays inside its pane once asked for.
    state.view.show_perf = true;
    let backend = RecordingBackend::default();
    let ctx = egui::Context::default();
    let mut t = 0.0;
    let mut frame = |state: &mut SharedState| {
        t += 1.0 / 60.0;
        let raw =
            egui::RawInput { screen_rect: Some(screen), time: Some(t), ..Default::default() };
        ctx.run_ui(raw, |ui| root_ui(ui, state, &backend, t))
    };
    frame(&mut state);
    let output = frame(&mut state);

    let plate = egui::Color32::from_black_alpha(0xC0);
    let hud = output
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) if rect.fill == plate => Some(rect.rect),
            _ => None,
        })
        .expect("the overlay should paint its backing plate");
    let area = perf_overlay_area(&state, screen, 1.0);
    assert!(
        area.contains_rect(hud),
        "the HUD ran {:.0}pt past its {:.0}pt pane: {hud:?} in {area:?}",
        hud.right() - area.right(),
        area.width(),
    );
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
    let fresh = SharedState::new(TextureFormat::Bgra8Unorm);
    assert!(!fresh.view.show_perf, "a fresh install opens with the overlay off");

    // A blob from before the setting existed: the key cut out of a saved one.
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.view.show_perf = true;
    let saved = state.save_persist();
    let old = saved.replacen("show_perf:true,", "", 1);
    assert_ne!(old, saved, "the show_perf cut must land for this to test anything");

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&old);
    assert!(!restored.view.show_perf, "a pre-show_perf blob opens with the overlay off");

    // And a project that asked for it still gets it: the cut above is what
    // makes the blob old, not the value, so the round-trip has to still work.
    let mut kept = SharedState::new(TextureFormat::Bgra8Unorm);
    kept.load_persist(&saved);
    assert!(kept.view.show_perf, "a project that turned the overlay on keeps it");
}
