use super::*;

#[test]
fn folds_preserve_other_picture_dimensions_at_the_requested_window() {
    for position in [Position::Right, Position::Below] {
        for section in Section::ALL {
            let mut layout = Layout { position, ..Layout::default() };
            let original = layout.rects(
                Rect::from_min_size(egui::Pos2::ZERO, layout.natural_size(24.0, 3.0)),
                24.0,
                3.0,
            );
            layout.folded[section as usize] = true;
            let size = layout.natural_size(24.0, 3.0);
            layout.fit(size, 24.0, 3.0);
            let folded = layout.rects(Rect::from_min_size(egui::Pos2::ZERO, size), 24.0, 3.0);
            for picture in [Section::Lattice, Section::Analyzer] {
                if picture != section {
                    assert_eq!(folded[picture as usize].size(), original[picture as usize].size());
                }
            }
        }
    }
}

#[test]
fn manual_resizing_does_not_change_a_hidden_pictures_remembered_size() {
    for position in [Position::Right, Position::Below] {
        let mut layout = Layout { position, folded: [false, true, false], ..Layout::default() };
        let before = layout.sizes().analyzer;
        layout.fit(vec2(1300.0, 900.0), 24.0, 3.0);
        assert_eq!(layout.sizes().analyzer, before);
        layout.folded[1] = false;
        let size = layout.natural_size(24.0, 3.0);
        let rects = layout.rects(Rect::from_min_size(egui::Pos2::ZERO, size), 24.0, 3.0);
        assert_eq!(
            if position == Position::Right { rects[1].width() } else { rects[1].height() },
            before
        );
    }
}

#[test]
fn persisted_layout_defaults_and_sanitization_are_local() {
    let mut layout: Layout =
        ron::from_str("(position:Below,below:(lattice:NaN),settings_tab:Lattice)").unwrap();
    layout.sanitize();
    assert_eq!(layout.position, Position::Below);
    assert_eq!(layout.settings_tab, Tab::Tuning);
    assert!(layout.below.lattice.is_finite());
    layout.right.analyzer = 80.0;
    layout.sanitize();
    assert_eq!(layout.right.analyzer, 80.0, "a legal small live pane must survive reopening");
}

#[test]
fn folding_a_region_keeps_the_analyzer_at_its_drag_minimum() {
    let min = theme::min_pane(1.0);
    let mut layout = Layout::default();
    assert!(layout.right.analyzer - 130.0 < min, "fixture must cross the minimum");
    layout.resize_region(1, Some(130.0), min);
    assert!(layout.right.analyzer >= min, "fold left an undraggable analyzer");
}

#[test]
fn a_window_resize_scales_the_width_held_by_a_folded_region() {
    let rail = theme::tab_bar_height(1.0);
    let gap = 3.0;
    for tab in [Tab::Spectral, Tab::Spiral] {
        let mut layout = Layout { analyzer_tab: tab, ..Layout::default() };
        let min = theme::min_pane(1.0);
        layout.resize_region(1, Some(60.0), min);
        let before = layout.right.analyzer;
        let held = layout.region_widths[1];
        assert!(held > 0.0, "fixture must hold width for the folded region");
        let area = layout.natural_size(rail, gap) + vec2(300.0, 0.0);
        layout.fit(area, rail, gap);
        let scale = layout.right.analyzer / before;
        assert!(scale > 1.1, "{tab:?}: fixture must enlarge the analyzer");
        assert!((layout.region_widths[1] - held * scale).abs() < 0.1, "{tab:?}");
        assert!((layout.natural_size(rail, gap).x - area.x).abs() < 0.1, "{tab:?}");
        layout.resize_region(1, None, min);
        assert!((layout.right.analyzer - (before + held) * scale).abs() < 0.1, "{tab:?}");
    }
}

#[test]
fn with_the_tab_bars_hidden_one_open_section_fills_the_window() {
    // No rail: a folded section draws nothing, so a gap beside it would be a
    // bare divider at the window's edge.
    for position in [Position::Right, Position::Below] {
        for open in Section::ALL {
            let mut layout = Layout { position, folded: [true; 3], ..Layout::default() };
            layout.folded[open as usize] = false;
            let size = layout.natural_size(0.0, 3.0);
            layout.fit(size, 0.0, 3.0);
            let area = Rect::from_min_size(egui::Pos2::ZERO, size);
            let rects = layout.rects(area, 0.0, 3.0);
            assert_eq!(rects[open as usize], area, "{position:?} {open:?}");
        }
    }
}
