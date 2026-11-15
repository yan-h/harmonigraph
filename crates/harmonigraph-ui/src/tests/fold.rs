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
        assert!(restored.install_document(&saved));
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

fn click_settled(h: &mut DockHarness, state: &mut SharedState, at: egui::Pos2) {
    click_at(h, state, at);
    h.settle_folds(state);
}

fn texts(output: &egui::FullOutput, label: &str) -> Vec<egui::Rect> {
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
            }
            _ => None,
        })
        .collect()
}

/// Dock the analyzer from its row on the Analyzer settings page.
fn dock(h: &mut DockHarness, state: &mut SharedState, position: Position) {
    state.workspace.layout.select(panes::Tab::AnalyzerSettings);
    let mut output = h.frame(state, vec![]);
    // Picture controls above View can put Dock below the viewport, or leave
    // it drawn but clipped at the pane's lower edge where a click misses it.
    for _ in 0..12 {
        let body = pane_body(state, &panes::Tab::AnalyzerSettings).unwrap();
        if texts(&output, "Dock").first().is_some_and(|row| body.contains_rect(*row)) {
            break;
        }
        let at = body.center();
        h.frame(
            state,
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(0.0, -100.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        output = h.settle_folds(state);
    }
    let row = texts(&output, "Dock").first().copied().expect("no Dock row");
    let label = if position == Position::Right { "Right" } else { "Below" };
    let at = texts(&output, label)
        .into_iter()
        .find(|rect| (rect.center().y - row.center().y).abs() < 4.0)
        .unwrap_or_else(|| panic!("no {label:?} beside Dock"))
        .center();
    click_settled(h, state, at);
    assert_eq!(state.workspace.layout.position, position);
}

fn right_click(h: &mut DockHarness, state: &mut SharedState, at: egui::Pos2) {
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    h.frame(state, vec![egui::Event::PointerMoved(at)]);
    h.frame(state, vec![button(true)]);
    h.frame(state, vec![button(false)]);
}

#[test]
fn right_clicking_a_picture_opens_its_settings_page() {
    let mut state = fresh();
    let mut h = DockHarness::new();
    h.settle(&mut state);
    h.collapse_click(&mut state, panes::Tab::Tuning);
    let cases = [
        (panes::Tab::Lattice, "Lattice settings", panes::Tab::LatticeSettings),
        (panes::Tab::Spectral, "Analyzer settings", panes::Tab::AnalyzerSettings),
    ];
    for (picture, item, page) in cases {
        let at = if picture == panes::Tab::Spectral {
            h.spectral_grab(&state)
        } else {
            state.workspace.layout_runtime.rects[Section::Lattice as usize].center()
        };
        right_click(&mut h, &mut state, at);
        let output = h.frame(&mut state, vec![]);
        let entry = texts(&output, item).first().copied().unwrap_or_else(|| panic!("no {item:?}"));
        click_settled(&mut h, &mut state, entry.center());
        assert!(!state.workspace.layout.folded[Section::Settings as usize], "{item}");
        assert_eq!(state.workspace.layout.settings_tab, page, "{item}");
        // Back to where the next case starts: another tab, and closed again.
        state.workspace.layout.select(panes::Tab::Tuning);
        h.collapse_click(&mut state, panes::Tab::Tuning);
    }
}

/// A second right-click on the same picture moves its open menu rather than
/// closing it.
#[test]
fn right_clicking_again_moves_the_open_menu() {
    let mut state = fresh();
    let mut h = DockHarness::new();
    h.settle(&mut state);
    // The second click lands well clear of the first: the menu opens below and
    // right of the pointer, and a right-click on its own frame closes it, so a
    // second click within a point of the first tests nothing about moving.
    let first = h.spectral_grab_at(&state, 0.8);
    let second = first - egui::vec2(40.0, 0.0);
    right_click(&mut h, &mut state, first);
    let before = texts(&h.frame(&mut state, vec![]), "Analyzer settings");
    right_click(&mut h, &mut state, second);
    let after = texts(&h.frame(&mut state, vec![]), "Analyzer settings");
    assert_eq!((before.len(), after.len()), (1, 1), "the menu closed");
    assert!(((after[0].min - before[0].min) - (second - first)).length() < 1.0);
}

#[test]
fn the_position_picker_restores_each_arrangements_sizes() {
    let mut state = fresh();
    let mut h = DockHarness::new();
    h.settle(&mut state);
    let right = state.workspace.layout_runtime.rects;
    let window = h.screen.size();
    dock(&mut h, &mut state, Position::Below);
    let below = state.workspace.layout_runtime.rects;
    assert!(below[1].top() > below[0].bottom());
    near(egui::vec2(below[0].width(), below[1].width()), egui::Vec2::splat(below[0].width()));
    dock(&mut h, &mut state, Position::Right);
    near(h.screen.size(), window);
    for (now, was) in state.workspace.layout_runtime.rects.iter().zip(right) {
        near(now.size(), was.size());
    }
    dock(&mut h, &mut state, Position::Below);
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

/// The settings divider stops at the settings floor rather than at the
/// pictures' minimum: past it the column's plots would have to stack.
#[test]
fn the_settings_divider_stops_at_the_settings_floor() {
    for position in [Position::Right, Position::Below] {
        let mut state = fresh();
        state.workspace.layout.position = position;
        let mut h = DockHarness::new();
        h.settle(&mut state);
        let [lattice, analyzer, settings] = state.workspace.layout_runtime.rects;
        let picture = if position == Position::Below { lattice } else { analyzer };
        let origin = egui::pos2((picture.right() + settings.left()) * 0.5, settings.center().y);
        let past = origin + egui::vec2(2000.0, 0.0);
        h.frame(&mut state, vec![egui::Event::PointerMoved(origin)]);
        h.frame(&mut state, vec![press(origin, true)]);
        h.frame(&mut state, vec![egui::Event::PointerMoved(past)]);
        h.frame(&mut state, vec![press(past, false)]);
        let width = state.workspace.layout_runtime.rects[Section::Settings as usize].width();
        assert!(
            (width - theme::min_settings(1.0)).abs() < 0.1,
            "{position:?}: settings dragged to {width}"
        );
        // Saved there too, not only drawn there: a saved width under the floor
        // would draw at the floor all the same, and the drag back would have
        // to make up the difference before the divider moved at all.
        let layout = &state.workspace.layout;
        let saved =
            if position == Position::Below { layout.below.settings } else { layout.right.settings };
        assert!((saved - width).abs() < 0.1, "{position:?}: saved {saved}, drawn {width}");
    }
}

/// A window narrowed until the settings column sits on its floor, then widened
/// back, returns to the layout it started from: the floor is drawn, not saved.
#[test]
fn a_window_narrowed_past_the_settings_floor_widens_back_to_its_layout() {
    for position in [Position::Right, Position::Below] {
        let mut state = fresh();
        state.workspace.layout.position = position;
        let mut h = DockHarness::new();
        h.settle(&mut state);
        let window = h.screen;
        let before = state.workspace.layout_runtime.rects;
        h.screen.max.x = h.screen.min.x + shell::MIN_WINDOW_SIZE.x;
        h.settle(&mut state);
        let narrow = state.workspace.layout_runtime.rects[Section::Settings as usize].width();
        assert!((narrow - theme::min_settings(1.0)).abs() < 0.1, "{position:?}: {narrow}");
        h.screen = window;
        h.settle(&mut state);
        for (now, was) in state.workspace.layout_runtime.rects.iter().zip(before) {
            near(now.size(), was.size());
        }
    }
}

/// A window narrow enough that the settings column is drawn on its floor
/// rather than at its saved width, and still wide enough that a fold's request
/// clears the minimum window. At 720 the default layout's column would shrink
/// to about 203 in proportion.
fn narrowed_past_the_floor(position: Position) -> (DockHarness, SharedState) {
    let mut state = fresh();
    state.workspace.layout.position = position;
    let mut h = DockHarness::new();
    h.settle(&mut state);
    h.screen.max.x = h.screen.min.x + 720.0;
    h.settle(&mut state);
    let layout = &state.workspace.layout;
    let saved =
        if position == Position::Below { layout.below.settings } else { layout.right.settings };
    assert!(saved < theme::min_settings(1.0) - 10.0, "{position:?}: fixture saved {saved}");
    (h, state)
}

/// Where the floor has overridden the saved settings width, a divider still
/// moves with the pointer from the first point of a drag: the drag starts from
/// the layout as drawn, not from the saved sizes underneath it.
#[test]
fn in_a_window_narrowed_past_the_floor_the_dividers_keep_pace_with_the_pointer() {
    let cases = [
        (Position::Right, Section::Settings, -30.0),
        (Position::Below, Section::Settings, -30.0),
        (Position::Right, Section::Lattice, 30.0),
    ];
    for (position, section, delta) in cases {
        let (mut h, mut state) = narrowed_past_the_floor(position);
        let [lattice, analyzer, settings] = state.workspace.layout_runtime.rects;
        let (left, right) = match (section, position) {
            (Section::Lattice, _) => (lattice, analyzer),
            (_, Position::Below) => (lattice, settings),
            _ => (analyzer, settings),
        };
        let origin = egui::pos2((left.right() + right.left()) * 0.5, settings.center().y);
        let to = origin + egui::vec2(delta, 0.0);
        let saved_settings = |state: &SharedState| match position {
            Position::Right => state.workspace.layout.right.settings,
            Position::Below => state.workspace.layout.below.settings,
        };
        let untouched = saved_settings(&state);
        h.frame(&mut state, vec![egui::Event::PointerMoved(origin)]);
        h.frame(&mut state, vec![press(origin, true)]);
        h.frame(&mut state, vec![egui::Event::PointerMoved(to)]);
        h.frame(&mut state, vec![press(to, false)]);
        let after = state.workspace.layout_runtime.rects;
        let grew = after[section as usize].width()
            - [lattice, analyzer, settings][section as usize].width();
        assert!(
            (grew - delta.abs()).abs() < 0.1,
            "{position:?}/{section:?}: a {delta}pt drag moved the divider {grew}pt"
        );
        // Trading width between the pictures leaves the settings column's own
        // saved width alone, so a wider window still gives it back.
        if section == Section::Lattice {
            assert_eq!(saved_settings(&state), untouched, "the pictures' divider saved the floor");
        }
    }
}

/// Folding one picture in a window narrowed past the floor leaves the other
/// drawn at the size it had, as it does in a roomy window.
#[test]
fn in_a_window_narrowed_past_the_floor_a_fold_keeps_the_other_pictures_size() {
    for position in [Position::Right, Position::Below] {
        for (tab, other) in
            [(panes::Tab::Lattice, Section::Analyzer), (panes::Tab::Spectral, Section::Lattice)]
        {
            let (mut h, mut state) = narrowed_past_the_floor(position);
            let was = state.workspace.layout_runtime.rects[other as usize].size();
            h.collapse_click(&mut state, tab);
            assert!(collapsed(&state, tab), "{position:?} {tab:?} failed to fold");
            near(state.workspace.layout_runtime.rects[other as usize].size(), was);
        }
    }
}

#[test]
fn dragging_a_divider_after_a_refused_unfold_keeps_unsqueezed_saved_sizes() {
    let mut state = fresh();
    let mut h = DockHarness::new();
    h.settle(&mut state);
    h.collapse_click(&mut state, panes::Tab::Tuning);
    let rail = state.workspace.layout_runtime.rects[Section::Settings as usize];
    click_at(&mut h, &mut state, rail.center());
    let requested = state.workspace.take_window_size_change().expect("unfold requests a resize");
    assert!(requested.x > 30.0, "fixture must request a wider window");
    // The host refuses that request, so the drawing fits while the saved
    // layout retains the size the fold just restored.
    h.settle(&mut state);
    let saved = state.workspace.layout.right;
    let [lattice, analyzer, settings] = state.workspace.layout_runtime.rects;
    assert!(settings.width() < saved.settings - 5.0, "fixture must draw squeezed sizes");
    let origin = egui::pos2((lattice.right() + analyzer.left()) * 0.5, lattice.center().y);
    h.frame(&mut state, vec![egui::Event::PointerMoved(origin)]);
    h.frame(&mut state, vec![press(origin, true)]);
    h.frame(&mut state, vec![egui::Event::PointerMoved(origin - egui::vec2(30.0, 0.0))]);
    h.frame(&mut state, vec![press(origin - egui::vec2(30.0, 0.0), false)]);
    assert!(state.workspace.layout.right.lattice < saved.lattice, "divider did not move");
    assert!(
        (state.workspace.layout_runtime.rects[0].width() - (lattice.width() - 30.0)).abs() < 1.0,
        "divider did not follow the pointer in the squeezed drawing"
    );
    assert!((state.workspace.layout.right.settings - saved.settings).abs() < 0.1);
    let saved_blob = state.save_persist();
    let mut reopened = fresh();
    assert!(reopened.install_document(&saved_blob));
    assert!((reopened.workspace.layout.right.settings - saved.settings).abs() < 0.1);
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

/// The fresh state with the analyzer's split at an even three quarters, so
/// each region is wider than the button rail it folds to by enough for a
/// fold to show in the window's width. The fresh split leaves the spectrum
/// only a few points wider than its rail.
fn split_state() -> SharedState {
    let mut state = fresh();
    state.picture.appearance.spectrum.roll_fraction = 0.75;
    state
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
            let mut state = split_state();
            state.picture.appearance.spectrum.orientation = orientation;
            let mut h = DockHarness::new();
            h.settle(&mut state);
            let before = [panes::Tab::Lattice, panes::Tab::Spectral, panes::Tab::Tuning]
                .map(|tab| pane_body(&state, &tab).unwrap().width());
            let appearance = state.picture.appearance.serialize();
            let window = h.screen.width();
            for index in [first, 1 - first] {
                region_click(&mut h, &mut state, index);
                assert_eq!(state.workspace.take_window_size_change(), None, "a region fold asked");
                h.settle(&mut state);
                assert!(state.workspace.interaction.analyzer_regions.collapsed[index]);
                // The fold keeps its space inside the Analyzer: no section moves.
                for (tab, width) in [panes::Tab::Lattice, panes::Tab::Spectral, panes::Tab::Tuning]
                    .into_iter()
                    .zip(before)
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
                h.settle(&mut state);
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
fn section_and_region_chevrons_show_hover_feedback() {
    let mut state = fresh();
    let mut h = DockHarness::new();
    h.settle(&mut state);
    let section = state.workspace.layout_runtime.rects[Section::Analyzer as usize];
    let section_cell =
        egui::Rect::from_min_size(section.min, egui::Vec2::splat(theme::tab_bar_height(1.0)));
    let region_cell = h.ctx.read_response(egui::Id::new(("analyzer region fold", 0))).unwrap().rect;
    for cell in [section_cell, region_cell] {
        let out = h.frame(&mut state, vec![egui::Event::PointerMoved(cell.center())]);
        assert!(
            out.shapes.iter().any(|cs| matches!(&cs.shape,
                egui::Shape::Rect(r)
                    if r.rect.contains(cell.center()) && r.fill == theme::widget_hover()
            )),
            "no hover fill at {cell:?}"
        );
        let chevrons = out
            .shapes
            .iter()
            .filter(|cs| {
                matches!(&cs.shape,
                    egui::Shape::Path(p)
                        if p.points.len() == 3
                            && cell.contains_rect(egui::Rect::from_points(&p.points))
                )
            })
            .count();
        // Two: a pane's fold is a double chevron, where a settings heading's
        // single one says something else (see `paint_fold`).
        assert_eq!(chevrons, 2, "not a double chevron at {cell:?}");
    }
}

#[test]
fn a_narrow_analyzer_keeps_one_header_row_and_its_spiral_choice() {
    for scale in [1.0, 1.5] {
        let mut state = fresh();
        state.workspace.layout.right =
            workspace::Sizes { lattice: 650.0, analyzer: 60.0, cross: 800.0, settings: 287.0 };
        state.workspace.layout.sized = true;
        let mut h = DockHarness::scaled(egui::vec2(1000.0, 800.0) * scale, scale, &mut state);
        h.settle(&mut state);
        let out = h.frame(&mut state, vec![]);
        let section = state.workspace.layout_runtime.rects[Section::Analyzer as usize];
        let body = pane_body(&state, &panes::Tab::Spectral).unwrap();
        assert!((body.top() - section.top() - theme::tab_bar_height(scale)).abs() < 0.5);
        let find = |out: &egui::FullOutput, name: &str| {
            out.shapes.iter().find_map(|cs| match &cs.shape {
                egui::Shape::Text(t)
                    if t.galley.text() == name
                        && t.pos.x >= section.left()
                        && t.pos.x < section.right() =>
                {
                    Some(egui::Rect::from_min_size(t.pos, t.galley.size()).center())
                }
                _ => None,
            })
        };
        // Too narrow for even one tab beside the overflow button, so the strip
        // is one dropdown carrying the current view's name.
        assert!(find(&out, "Spiral").is_none(), "fixture did not reach the dropdown");
        let picker = find(&out, "Analyzer").expect("current view dropdown");
        click_at(&mut h, &mut state, picker);
        let menu = h.frame(&mut state, vec![]);
        let spiral = find(&menu, "Spiral").expect("Spiral in dropdown");
        click_at(&mut h, &mut state, spiral);
        assert_eq!(state.workspace.layout.analyzer_tab, panes::Tab::Spiral);
    }
}

/// Where a region's outer edge survives its fold, Expand appears exactly where
/// Collapse was.
#[test]
fn a_region_expand_button_takes_the_place_of_its_collapse_button() {
    for (orientation, index) in [(SpectralOrientation::Right, 1), (SpectralOrientation::Left, 0)] {
        let mut state = split_state();
        state.picture.appearance.spectrum.orientation = orientation;
        let mut h = DockHarness::new();
        h.settle(&mut state);
        let id = egui::Id::new(("analyzer region fold", index));
        let button = |h: &mut DockHarness, state: &mut SharedState| {
            let cell = h.ctx.read_response(id).expect("region control is drawn").rect;
            // The clicked control keeps focus, so it may still draw hot.
            h.frame(state, vec![egui::Event::PointerGone])
                .shapes
                .iter()
                .find_map(|cs| match &cs.shape {
                    egui::Shape::Rect(r)
                        if cell.contains_rect(r.rect)
                            && [theme::widget(), theme::widget_hover()].contains(&r.fill)
                            && (r.rect.height() - theme::row_height(1.0)).abs() < 0.5 =>
                    {
                        Some(r.rect)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{orientation:?} drew no button in {cell:?}"))
        };
        let collapse = button(&mut h, &mut state);
        region_click(&mut h, &mut state, index);
        h.settle(&mut state);
        assert!(state.workspace.interaction.analyzer_regions.collapsed[index]);
        let expand = button(&mut h, &mut state);
        near(expand.min.to_vec2(), collapse.min.to_vec2());
        near(expand.size(), collapse.size());
    }
}

/// With a tape, the Analyzer's header holds a Tape tab between its pictures,
/// even in the narrow column the fresh layout gives it. Tape mode lays the
/// audition's controls and the overview along the pane's bottom, and leaving
/// for Spiral leaves Tape mode.
#[test]
fn the_tape_is_a_tab_of_the_analyzer_and_lays_its_controls_and_overview_below() {
    let mut state = fresh();
    // A narrow column, set here rather than inherited from the fresh layout:
    // room for the three tabs and little more.
    state.workspace.layout.position = Position::Right;
    state.workspace.layout.right.analyzer = 200.0;
    let mut h = DockHarness::new();
    h.settle(&mut state);
    let section = state.workspace.layout_runtime.rects[Section::Analyzer as usize];
    let header = egui::Rect::from_min_size(
        section.min,
        egui::vec2(section.width(), theme::tab_bar_height(1.0)),
    );
    let at = |out: &egui::FullOutput, name: &str, place: egui::Rect| {
        out.shapes
            .iter()
            .find_map(|cs| match &cs.shape {
                egui::Shape::Text(t) if t.galley.text() == name => {
                    Some(egui::Rect::from_min_size(t.pos, t.galley.size()).center())
                }
                _ => None,
            })
            .filter(|at| place.contains(*at))
    };
    let out = h.frame(&mut state, vec![]);
    assert!(at(&out, "Tape", header).is_none(), "a tab for a tape the plugin lacks");

    state.picture.runtime.tape.present = true;
    state.picture.runtime.tape.enabled = true;
    let overview = egui::Id::new(("tape-overview", panes::DOCKED_SURFACE));
    let out = h.frame(&mut state, vec![]);
    let tape = at(&out, "Tape", header).expect("the Tape tab is on the header");
    assert!(at(&out, "Spiral", header).is_some(), "the Tape tab pushed Spiral off the strip");
    assert!(at(&out, "Stop", section).is_none(), "the live picture has no audition controls");
    assert!(h.ctx.read_response(overview).is_none(), "the live picture has no overview");

    click_at(&mut h, &mut state, tape);
    assert!(state.picture.runtime.tape.showing, "the tab did not reach Tape mode");
    assert_eq!(state.workspace.layout.analyzer_tab, panes::Tab::Spectral);
    let out = h.frame(&mut state, vec![]);
    let bottom = egui::Rect::from_min_max(section.center(), section.max);
    let lower = egui::Rect::from_x_y_ranges(section.x_range(), bottom.y_range());
    assert!(at(&out, "Stop", lower).is_some(), "Tape mode's controls are not along the bottom");
    let strip = h.ctx.read_response(overview).expect("Tape mode lays the overview").rect;
    assert!((strip.bottom() - section.bottom()).abs() < 0.5, "{strip:?} in {section:?}");

    let spiral = at(&out, "Spiral", header).expect("the Spiral tab");
    click_at(&mut h, &mut state, spiral);
    assert_eq!(state.workspace.layout.analyzer_tab, panes::Tab::Spiral);
    assert!(!state.picture.runtime.tape.showing, "Spiral left the analyzer in Tape mode");
}

/// A short Analyzer gives up the overview before the controls, which hold
/// Stop.
#[test]
fn a_short_analyzer_keeps_the_tape_controls_and_drops_the_overview() {
    let mut state = fresh();
    state.workspace.layout.position = Position::Below;
    state.workspace.layout.below.analyzer = 150.0;
    state.picture.runtime.tape.present = true;
    state.picture.runtime.tape.enabled = true;
    state.picture.runtime.tape.showing = true;
    let mut h = DockHarness::new();
    h.settle(&mut state);
    let body = pane_body(&state, &panes::Tab::Spectral).expect("the Analyzer is drawn");
    let (row, strip) = (theme::tab_bar_height(1.0), 1.5 * theme::tab_bar_height(1.0));
    assert!(
        body.height() > 3.0 * row && body.height() <= 3.0 * (row + strip),
        "fixture misses the band between the two cutoffs: {body:?}"
    );
    let out = h.frame(&mut state, vec![]);
    let stop = out.shapes.iter().any(|cs| {
        matches!(&cs.shape, egui::Shape::Text(t) if t.galley.text() == "Stop" && body.contains(t.pos))
    });
    assert!(stop, "the controls went with the overview");
    let overview = egui::Id::new(("tape-overview", panes::DOCKED_SURFACE));
    assert!(h.ctx.read_response(overview).is_none(), "the overview stayed on a short pane");
}
