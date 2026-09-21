//! Fixed-section folding through the real controls and window resize handshake.
use super::harness::*;
use crate::*;
use workspace::{Position, Section};

fn near(a: egui::Vec2, b: egui::Vec2) {
    assert!((a - b).length() < 0.1, "{a:?} != {b:?}");
}

#[test]
fn each_section_folds_independently_in_both_arrangements() {
    for position in [Position::Right, Position::Below] {
        for tab in [panes::Tab::Lattice, panes::Tab::Spectral, panes::Tab::Tuning] {
            let mut state = fresh();
            state.workspace.layout.position = position;
            let mut h = DockHarness::new();
            h.settle(&mut state);
            let before = state.workspace.layout_runtime.rects;
            let window = h.screen.size();
            h.collapse_click(&mut state, tab);
            assert!(collapsed(&state, tab), "{position:?} {tab:?} failed to fold");
            for other in [panes::Tab::Lattice, panes::Tab::Spectral] {
                if other != tab {
                    let section = Section::of(other) as usize;
                    near(
                        state.workspace.layout_runtime.rects[section].size(),
                        before[section].size(),
                    );
                }
            }
            let delta = window - h.screen.size();
            if position == Position::Below && tab.is_picture() {
                assert!(delta.y > 100.0);
                assert!(delta.x.abs() < 0.1);
            } else {
                assert!(delta.x > 100.0);
                assert!(delta.y.abs() < 0.1);
            }
            h.collapse_click(&mut state, tab);
            assert!(!collapsed(&state, tab));
            near(h.screen.size(), window);
            for (now, was) in state.workspace.layout_runtime.rects.iter().zip(before) {
                near(now.size(), was.size());
            }
        }
    }
}

#[test]
fn independent_picture_and_settings_folds_leave_a_reopen_control() {
    for position in [Position::Right, Position::Below] {
        let mut state = fresh();
        state.workspace.layout.position = position;
        let mut h = DockHarness::new();
        h.settle(&mut state);
        let window = h.screen.size();
        for tab in [panes::Tab::Spectral, panes::Tab::Tuning] {
            h.collapse_click(&mut state, tab);
        }
        assert!(state.workspace.layout.visible(panes::Tab::Lattice));
        assert!(state.workspace.layout.folded[1] && state.workspace.layout.folded[2]);
        h.collapse_click(&mut state, panes::Tab::Spectral);
        h.collapse_click(&mut state, panes::Tab::Lattice);
        assert!(state.workspace.layout.visible(panes::Tab::Spectral));
        h.collapse_click(&mut state, panes::Tab::Tuning);
        assert!(state.workspace.layout.visible(panes::Tab::Tuning));
        h.collapse_click(&mut state, panes::Tab::Lattice);
        near(h.screen.size(), window);
    }
}

#[test]
fn saved_fold_sizes_survive_a_new_context_and_a_constrained_window() {
    for position in [Position::Right, Position::Below] {
        let mut state = fresh();
        state.workspace.layout.position = position;
        let mut h = DockHarness::new();
        h.settle(&mut state);
        let original = h.screen.size();
        h.collapse_click(&mut state, panes::Tab::Spectral);
        let saved = state.save_persist();
        let mut restored = fresh();
        assert!(restored.load_persist(&saved));
        let mut reopened = DockHarness::at(h.screen.size().max(egui::vec2(900.0, 700.0)));
        reopened.settle(&mut restored);
        // The extra width/height on reopen is a host constraint, not a new dial.
        h = reopened;
        h.collapse_click(&mut restored, panes::Tab::Spectral);
        near(h.screen.size(), original);
    }
}

#[test]
fn resetting_layout_restores_open_sections_after_traversal() {
    let mut state = fresh();
    let mut h = DockHarness::new();
    h.settle(&mut state);
    h.collapse_click(&mut state, panes::Tab::Lattice);
    state.workspace.interaction.reset_layout = true;
    h.settle_folds(&mut state);
    assert_eq!(state.workspace.layout.folded, [false; 3]);
    assert_eq!(state.workspace.layout.position, Position::Right);
}

fn click_label(h: &mut DockHarness, state: &mut SharedState, label: &str) {
    let output = h.frame(state, vec![]);
    let at = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(egui::Rect::from_min_size(text.pos, text.galley.size()).center())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no {label:?} control"));
    h.frame(state, vec![egui::Event::PointerMoved(at)]);
    h.frame(state, vec![press(at, true)]);
    h.frame(state, vec![press(at, false)]);
    h.settle_folds(state);
}

#[test]
fn the_position_picker_restores_each_arrangements_sizes() {
    let mut state = fresh();
    let mut h = DockHarness::new();
    h.settle(&mut state);
    let right = state.workspace.layout_runtime.rects;
    let window = h.screen.size();
    click_label(&mut h, &mut state, "Layout");
    click_label(&mut h, &mut state, "Below");
    assert_eq!(state.workspace.layout.position, Position::Below);
    let below = state.workspace.layout_runtime.rects;
    assert!(below[1].top() > below[0].bottom());
    near(egui::vec2(below[0].width(), below[1].width()), egui::Vec2::splat(below[0].width()));
    click_label(&mut h, &mut state, "Layout");
    click_label(&mut h, &mut state, "Right");
    near(h.screen.size(), window);
    for (now, was) in state.workspace.layout_runtime.rects.iter().zip(right) {
        near(now.size(), was.size());
    }
    click_label(&mut h, &mut state, "Layout");
    click_label(&mut h, &mut state, "Below");
    for (now, was) in state.workspace.layout_runtime.rects.iter().zip(below) {
        near(now.size(), was.size());
    }
}

#[test]
fn dividers_follow_the_pointer_in_both_arrangements() {
    for position in [Position::Right, Position::Below] {
        let mut state = fresh();
        state.workspace.layout.position = position;
        let mut h = DockHarness::new();
        h.settle(&mut state);
        let [lattice, analyzer, _] = state.workspace.layout_runtime.rects;
        let below = position == Position::Below;
        let origin = if below {
            egui::pos2(lattice.center().x, (lattice.bottom() + analyzer.top()) * 0.5)
        } else {
            egui::pos2((lattice.right() + analyzer.left()) * 0.5, lattice.center().y)
        };
        let direction = if below { egui::Vec2::Y } else { egui::Vec2::X };
        h.frame(&mut state, vec![egui::Event::PointerMoved(origin)]);
        h.frame(&mut state, vec![press(origin, true)]);
        for delta in [30.0, 60.0] {
            h.frame(&mut state, vec![egui::Event::PointerMoved(origin + direction * delta)]);
        }
        h.frame(&mut state, vec![]);
        let moved = state.workspace.layout_runtime.rects;
        near(moved[0].size(), lattice.size() + direction * 60.0);
        near(moved[1].size(), analyzer.size() - direction * 60.0);
        // Going past the floor and partway back still owes the overshoot.
        h.frame(&mut state, vec![egui::Event::PointerMoved(origin + direction * 2000.0)]);
        h.frame(&mut state, vec![egui::Event::PointerMoved(origin + direction * 1000.0)]);
        h.frame(&mut state, vec![]);
        let span = state.workspace.layout_runtime.rects[1].size().dot(direction);
        assert!((span - theme::min_pane(1.0)).abs() < 0.1);
        h.frame(&mut state, vec![egui::Event::PointerMoved(origin)]);
        h.frame(&mut state, vec![press(origin, false)]);
        near(state.workspace.layout_runtime.rects[0].size(), lattice.size());
        near(state.workspace.layout_runtime.rects[1].size(), analyzer.size());
    }
}

#[test]
fn stacked_settings_only_is_compact_and_all_sections_can_reopen() {
    let mut state = fresh();
    state.workspace.layout.position = Position::Below;
    state.workspace.min_window_size = shell::MIN_WINDOW_SIZE;
    let mut h = DockHarness::new();
    h.settle(&mut state);
    let window = h.screen.size();
    h.collapse_click(&mut state, panes::Tab::Lattice);
    h.collapse_click(&mut state, panes::Tab::Spectral);
    assert_eq!(h.screen.size(), shell::MIN_WINDOW_SIZE);
    assert!(state.workspace.layout.visible(panes::Tab::Tuning));
    for rect in &state.workspace.layout_runtime.rects[..2] {
        assert_eq!(rect.width(), theme::tab_bar_height(1.0));
    }
    h.collapse_click(&mut state, panes::Tab::Tuning);
    assert_eq!(state.workspace.layout.folded, [true; 3]);
    for tab in [panes::Tab::Tuning, panes::Tab::Spectral, panes::Tab::Lattice] {
        h.collapse_click(&mut state, tab);
    }
    near(h.screen.size(), window);
    assert_eq!(state.workspace.layout.folded, [false; 3]);
}

#[test]
fn a_discarded_pass_does_not_toggle_a_fold_twice() {
    let mut state = fresh();
    let mut h = DockHarness::new();
    h.settle(&mut state);
    let rect = state.workspace.layout_runtime.rects[1];
    let at = rect.min + egui::vec2(12.0, theme::tab_bar_height(1.0) * 0.5);
    h.frame(&mut state, vec![egui::Event::PointerMoved(at)]);
    h.frame(&mut state, vec![press(at, true)]);
    let ctx = h.ctx.clone();
    let raw = egui::RawInput {
        screen_rect: Some(h.screen),
        events: vec![press(at, false)],
        ..Default::default()
    };
    let output = ctx.run_ui(raw, |ui| {
        root_ui(ui, &mut state, &RecordingBackend::default(), 1.0);
        if ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("exercise repeated layout pass");
        }
    });
    assert!(output.platform_output.num_completed_passes > 1);
    assert!(state.workspace.layout.folded[1]);
    assert_eq!(
        state.workspace.layout_runtime.rects[1], rect,
        "the click's frame retains its geometry until the host answers"
    );
    assert!(state.workspace.take_window_size_change().unwrap().x < -100.0);
}

fn region_click(h: &mut DockHarness, state: &mut SharedState, index: usize) {
    let id = egui::Id::new(("analyzer region fold", index));
    let at = h.ctx.read_response(id).expect("region control is drawn").rect.center();
    click_at(h, state, at);
}

fn click_at(h: &mut DockHarness, state: &mut SharedState, at: egui::Pos2) {
    h.frame(state, vec![egui::Event::PointerMoved(at)]);
    h.frame(state, vec![egui::Event::PointerMoved(at), press(at, true)]);
    h.frame(state, vec![press(at, false)]);
}

#[test]
fn analyzer_region_buttons_sit_flush_in_opposite_outer_corners() {
    for (orientation, horizontal, first_at_low_end) in [
        (SpectralOrientation::Left, true, true),
        (SpectralOrientation::Right, true, false),
        (SpectralOrientation::Top, false, true),
        (SpectralOrientation::Bottom, false, false),
    ] {
        let mut state = fresh();
        state.picture.appearance.spectrum.orientation = orientation;
        let mut h = DockHarness::new();
        h.settle(&mut state);
        let body = pane_body(&state, &panes::Tab::Spectral).unwrap();
        let (low, high) =
            if horizontal { (body.left(), body.right()) } else { (body.top(), body.bottom()) };
        for index in 0..2 {
            let id = egui::Id::new(("analyzer region fold", index));
            let button = h.ctx.read_response(id).expect("region button is drawn").rect;
            let at_low_end = (index == 0) == first_at_low_end;
            let (button_edge, pane_edge) = if at_low_end {
                (if horizontal { button.left() } else { button.top() }, low)
            } else {
                (if horizontal { button.right() } else { button.bottom() }, high)
            };
            assert!(
                (button_edge - pane_edge).abs() < 1.0,
                "{orientation:?} region {index} button is not flush with the outer edge of {body:?}",
            );
            let (button_pitch_edge, pane_pitch_edge) = if horizontal {
                (button.top(), body.top())
            } else {
                (button.right(), body.right())
            };
            assert!(
                (button_pitch_edge - pane_pitch_edge).abs() < 1.0,
                "{orientation:?} region {index} button is not flush with the pitch edge",
            );
        }
    }
}

#[test]
fn collapsed_panes_open_from_the_middle_of_their_rails() {
    for tab in [panes::Tab::Tuning, panes::Tab::Lattice, panes::Tab::Spectral] {
        let mut state = fresh();
        let mut h = DockHarness::new();
        h.settle(&mut state);
        h.collapse_click(&mut state, tab);
        assert!(collapsed(&state, tab));
        let rail = state.workspace.layout_runtime.rects[Section::of(tab) as usize];
        assert!(
            rail.width() < 40.0 || rail.height() < 40.0,
            "{tab:?} did not settle to a rail: {rail:?}"
        );
        click_at(&mut h, &mut state, rail.center());
        h.settle_folds(&mut state);
        assert!(!collapsed(&state, tab), "{tab:?} did not open from its rail body");
    }
}

#[test]
fn analyzer_regions_fold_independently_and_restore_the_original_geometry() {
    for orientation in [SpectralOrientation::Left, SpectralOrientation::Right] {
        for first in [0, 1] {
            let mut state = fresh();
            state.picture.appearance.spectrum.orientation = orientation;
            let mut h = DockHarness::new();
            h.settle(&mut state);
            let before = [panes::Tab::Lattice, panes::Tab::Spectral, panes::Tab::Tuning]
                .map(|tab| pane_body(&state, &tab).unwrap().width());
            let appearance = state.picture.appearance.serialize();
            let window = h.screen.width();
            for index in [first, 1 - first] {
                region_click(&mut h, &mut state, index);
                assert!(
                    !state.workspace.interaction.analyzer_regions.collapsed[index],
                    "click frame retains its old picture"
                );
                h.settle_folds(&mut state);
                assert!(state.workspace.interaction.analyzer_regions.collapsed[index]);
                assert!(h.screen.width() < window - 10.0);
                for (tab, width) in
                    [(panes::Tab::Lattice, before[0]), (panes::Tab::Tuning, before[2])]
                {
                    assert!(
                        (pane_body(&state, &tab).unwrap().width() - width).abs() < 1.0,
                        "{tab:?} resized during {orientation:?} region {index} fold"
                    );
                }
            }
            // Folding the container preserves both independently folded regions.
            h.collapse_click(&mut state, panes::Tab::Spectral);
            h.collapse_click(&mut state, panes::Tab::Spectral);
            assert_eq!(state.workspace.interaction.analyzer_regions.collapsed, [true; 2]);
            for index in [first, 1 - first] {
                region_click(&mut h, &mut state, index);
                h.settle_folds(&mut state);
            }
            assert!((h.screen.width() - window).abs() < 1.0);
            for (tab, width) in [panes::Tab::Lattice, panes::Tab::Spectral, panes::Tab::Tuning]
                .into_iter()
                .zip(before)
            {
                assert!(
                    (pane_body(&state, &tab).unwrap().width() - width).abs() < 1.0,
                    "{orientation:?}, first {first}: {tab:?} did not return to {width}: {:?}",
                    pane_body(&state, &tab)
                );
            }
            assert_eq!(
                state.picture.appearance.serialize(),
                appearance,
                "folding never edits a video's appearance"
            );
        }
    }
}

#[test]
fn analyzer_region_restore_survives_a_saved_project() {
    let mut state = fresh();
    state.picture.appearance.spectrum.orientation = SpectralOrientation::Left;
    let mut h = DockHarness::new();
    h.settle(&mut state);
    let width = h.screen.width();
    let lattice = pane_body(&state, &panes::Tab::Lattice).unwrap().width();
    region_click(&mut h, &mut state, 1);
    h.settle_folds(&mut state);
    let saved = state.save_persist();
    let mut reopened = fresh();
    assert!(reopened.load_persist(&saved));
    h.settle(&mut reopened);
    assert!(
        (pane_body(&reopened, &panes::Tab::Lattice).unwrap().width() - lattice).abs() < 1.0,
        "saved layout retains neighboring widths"
    );
    assert_eq!(reopened.workspace.take_window_size_change(), None);
    assert_eq!(reopened.workspace.interaction.analyzer_regions.collapsed, [false, true]);
    region_click(&mut h, &mut reopened, 1);
    h.settle_folds(&mut reopened);
    assert!((h.screen.width() - width).abs() < 1.0, "saved internal fold lost its restore ceiling");
}

#[test]
fn analyzer_region_restore_respects_a_split_edited_while_folded() {
    let mut state = fresh();
    state.picture.appearance.spectrum.orientation = SpectralOrientation::Left;
    let mut h = DockHarness::new();
    h.settle(&mut state);
    region_click(&mut h, &mut state, 1);
    h.settle_folds(&mut state);
    // The Video preview edits this shared appearance dial while the editor's
    // own divider is hidden. Its new value must invalidate the old hold.
    state.picture.appearance.spectrum.roll_fraction = 0.35;
    h.settle(&mut state);
    region_click(&mut h, &mut state, 1);
    h.settle_folds(&mut state);
    let body = pane_body(&state, &panes::Tab::Spectral).unwrap();
    let split =
        h.ctx.read_response(egui::Id::new(("spectral-split", 0usize))).unwrap().rect.center().x;
    assert!(((split - body.left()) / body.width() - 0.65).abs() < 0.01);
}

#[test]
fn analyzer_region_folds_remain_usable_when_the_host_constrains_the_window() {
    for floor in [0.0, 950.0] {
        let mut state = fresh();
        state.picture.appearance.spectrum.orientation = SpectralOrientation::Left;
        state.workspace.min_window_size.x = floor;
        let mut h = DockHarness::new();
        h.settle(&mut state);
        for index in [0, 1, 0, 1] {
            region_click(&mut h, &mut state, index);
            if floor == 0.0 {
                // A host that rejects the request entirely.
                state.workspace.take_window_size_change();
                h.settle(&mut state);
            } else {
                h.settle_folds(&mut state);
            }
            assert_eq!(
                state.workspace.take_window_size_change(),
                None,
                "no repeated resize requests"
            );
        }
        assert_eq!(state.workspace.interaction.analyzer_regions.collapsed, [false; 2]);
        assert!((h.screen.width() - 1000.0).abs() < 1.0);
    }
}

#[test]
fn analyzer_local_region_folds_restore_the_original_split() {
    for orientation in [
        SpectralOrientation::Top,
        SpectralOrientation::Bottom,
        SpectralOrientation::Left,
        SpectralOrientation::Right,
    ] {
        for sequence in [&[0, 0][..], &[1, 1], &[0, 1, 0, 1], &[1, 0, 1, 0]] {
            let mut state = fresh();
            state.picture.appearance.spectrum.orientation = orientation;
            if !orientation.is_time_vertical() {
                // A stacked Analyzer also folds locally, without resizing the
                // window. Exercise the same geometry in both depth axes.
                state.workspace.layout.position = Position::Below;
            }
            let mut h = DockHarness::new();
            h.settle(&mut state);
            let lattice = pane_body(&state, &panes::Tab::Lattice).unwrap();
            let id = egui::Id::new(("spectral-split", 0usize));
            let before = h.ctx.read_response(id).unwrap().rect.center();
            for &index in sequence {
                region_click(&mut h, &mut state, index);
                h.settle_folds(&mut state);
                assert_eq!(pane_body(&state, &panes::Tab::Lattice).unwrap(), lattice);
                assert_eq!(h.screen.width(), 1000.0);
            }
            assert_eq!(state.workspace.interaction.analyzer_regions.collapsed, [false; 2]);
            let after = h.ctx.read_response(id).unwrap().rect.center();
            assert!(
                before.distance(after) < 1.0,
                "{orientation:?} {sequence:?}: {before:?} -> {after:?}"
            );
        }
    }
}

#[test]
fn analyzer_region_restore_repays_its_width_after_orientation_changes() {
    for index in [0, 1] {
        for roundtrip in [false, true] {
            let mut state = fresh();
            state.picture.appearance.spectrum.orientation = SpectralOrientation::Left;
            let mut h = DockHarness::new();
            h.settle(&mut state);
            let before = pane_body(&state, &panes::Tab::Spectral).unwrap();
            let id = egui::Id::new(("spectral-split", 0usize));
            let split = h.ctx.read_response(id).unwrap().rect.center();
            region_click(&mut h, &mut state, index);
            h.settle_folds(&mut state);
            state.picture.appearance.spectrum.orientation = SpectralOrientation::Top;
            h.settle(&mut state);
            // Debt and restoration geometry must survive a save even while
            // the picture is using a different axis from the original fold.
            let saved = state.save_persist();
            state = fresh();
            assert!(state.load_persist(&saved));
            h.settle(&mut state);
            if roundtrip {
                state.picture.appearance.spectrum.orientation = SpectralOrientation::Left;
                h.settle(&mut state);
            }
            region_click(&mut h, &mut state, index);
            h.settle_folds(&mut state);
            assert!((h.screen.width() - 1000.0).abs() < 1.0);
            let after = pane_body(&state, &panes::Tab::Spectral).unwrap();
            assert!(
                (after.width() - before.width()).abs() <= 1.0,
                "region {index}, roundtrip {roundtrip}: {before:?} -> {after:?}, window {}",
                h.screen.width()
            );
            if roundtrip {
                assert!(h.ctx.read_response(id).unwrap().rect.center().distance(split) < 1.0);
            }
        }
    }
}

#[test]
fn resetting_the_layout_restores_space_held_by_analyzer_regions() {
    for fold_lattice_first in [None, Some(true), Some(false)] {
        let mut state = fresh();
        state.picture.appearance.spectrum.orientation = SpectralOrientation::Left;
        let mut h = DockHarness::new();
        h.settle(&mut state);
        if fold_lattice_first == Some(true) {
            h.collapse_click(&mut state, panes::Tab::Lattice);
        }
        region_click(&mut h, &mut state, 1);
        h.settle_folds(&mut state);
        if fold_lattice_first == Some(false) {
            h.collapse_click(&mut state, panes::Tab::Lattice);
        }
        assert!(h.screen.width() < 1000.0);
        let saved = state.save_persist();
        state = fresh();
        assert!(state.load_persist(&saved));
        h.settle(&mut state);
        state.workspace.reset_layout();
        h.frame(&mut state, vec![]);
        h.settle_folds(&mut state);
        assert_eq!(state.workspace.interaction.analyzer_regions.collapsed, [false; 2]);
        assert!(
            (h.screen.width() - 996.0).abs() < 1.0,
            "order {fold_lattice_first:?}: {}",
            h.screen.width()
        );
    }
}

#[test]
fn internal_fold_width_belongs_to_the_arrangement_that_removed_it() {
    for closed_in in [Position::Right, Position::Below] {
        let mut state = fresh();
        state.picture.appearance.spectrum.orientation = SpectralOrientation::Left;
        let mut h = DockHarness::new();
        h.settle(&mut state);
        let original = h.screen.size();
        let right = state.workspace.layout_runtime.rects;
        if closed_in == Position::Below {
            click_label(&mut h, &mut state, "Layout");
            click_label(&mut h, &mut state, "Below");
        }
        region_click(&mut h, &mut state, 1);
        h.settle_folds(&mut state);
        click_label(&mut h, &mut state, "Layout");
        click_label(
            &mut h,
            &mut state,
            if closed_in == Position::Right { "Below" } else { "Right" },
        );
        region_click(&mut h, &mut state, 1);
        h.settle_folds(&mut state);
        if closed_in == Position::Right {
            click_label(&mut h, &mut state, "Layout");
            click_label(&mut h, &mut state, "Right");
        }
        near(h.screen.size(), original);
        for (now, before) in state.workspace.layout_runtime.rects.iter().zip(right) {
            near(now.size(), before.size());
        }
        assert_eq!(state.workspace.layout.region_widths, [0.0; 2]);
    }
}

#[test]
fn a_discarded_pass_does_not_charge_an_internal_fold_twice() {
    let mut state = fresh();
    state.picture.appearance.spectrum.orientation = SpectralOrientation::Left;
    let mut h = DockHarness::new();
    h.settle(&mut state);
    let original = h.screen.size();
    let rect = state.workspace.layout_runtime.rects[1];
    let at =
        h.ctx.read_response(egui::Id::new(("analyzer region fold", 1usize))).unwrap().rect.center();
    h.frame(&mut state, vec![egui::Event::PointerMoved(at)]);
    h.frame(&mut state, vec![press(at, true)]);
    let output = h.ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(h.screen),
            time: Some(h.next_time()),
            events: vec![press(at, false)],
            ..Default::default()
        },
        |ui| {
            root_ui(ui, &mut state, &RecordingBackend::default(), 1.0);
            if ui.ctx().current_pass_index() == 0 {
                ui.ctx().request_discard("exercise internal fold repeat");
            }
        },
    );
    assert!(output.platform_output.num_completed_passes > 1);
    assert_eq!(state.workspace.layout_runtime.rects[1], rect);
    assert_eq!(state.workspace.interaction.analyzer_regions.collapsed, [false; 2]);
    h.settle_folds(&mut state);
    assert_eq!(state.workspace.interaction.analyzer_regions.collapsed, [false, true]);
    region_click(&mut h, &mut state, 1);
    h.settle_folds(&mut state);
    near(h.screen.size(), original);
}
