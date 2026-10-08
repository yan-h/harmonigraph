use super::*;

/// The settings floor at the design scale, which every fixture here is drawn at.
fn floor() -> f32 {
    theme::min_settings(1.0)
}

#[test]
fn folds_preserve_other_picture_dimensions_at_the_requested_window() {
    for position in [Position::Right, Position::Below] {
        for section in Section::ALL {
            let mut layout = Layout { position, ..Layout::default() };
            let original = layout.rects(
                Rect::from_min_size(egui::Pos2::ZERO, layout.natural_size(24.0, 3.0, floor())),
                24.0,
                3.0,
                floor(),
            );
            layout.folded[section as usize] = true;
            let size = layout.natural_size(24.0, 3.0, floor());
            layout.fit(size, 24.0, 3.0);
            let folded =
                layout.rects(Rect::from_min_size(egui::Pos2::ZERO, size), 24.0, 3.0, floor());
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
        let size = layout.natural_size(24.0, 3.0, floor());
        let rects = layout.rects(Rect::from_min_size(egui::Pos2::ZERO, size), 24.0, 3.0, floor());
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
        let area = layout.natural_size(rail, gap, floor()) + vec2(300.0, 0.0);
        layout.fit(area, rail, gap);
        let scale = layout.right.analyzer / before;
        assert!(scale > 1.1, "{tab:?}: fixture must enlarge the analyzer");
        assert!((layout.region_widths[1] - held * scale).abs() < 0.1, "{tab:?}");
        assert!((layout.natural_size(rail, gap, floor()).x - area.x).abs() < 0.1, "{tab:?}");
        layout.resize_region(1, None, min);
        assert!((layout.right.analyzer - (before + held) * scale).abs() < 0.1, "{tab:?}");
    }
}

#[test]
fn with_the_tab_bars_hidden_the_open_sections_fill_the_window() {
    // No rail: a folded section draws nothing, so a gap beside it would be a
    // bare divider at the window's edge, or a double one between the open
    // sections either side of it.
    for position in [Position::Right, Position::Below] {
        for open in 1..8_usize {
            let folded = [0, 1, 2].map(|index| open & (1 << index) == 0);
            let mut layout = Layout { position, folded, ..Layout::default() };
            let size = layout.natural_size(0.0, 3.0, floor());
            layout.fit(size, 0.0, 3.0);
            let area = Rect::from_min_size(egui::Pos2::ZERO, size);
            let rects = layout.rects(area, 0.0, 3.0, floor());
            let shown: Vec<Rect> =
                (0..3).filter(|&index| !folded[index]).map(|index| rects[index]).collect();
            let bounds = shown.iter().fold(Rect::NOTHING, |bounds, rect| bounds.union(*rect));
            assert_eq!(bounds, area, "{position:?} {folded:?}");
            let covered: f32 = shown.iter().map(|rect| rect.area()).sum();
            let gaps = match (position, shown.len()) {
                (Position::Right, n) => (n - 1) as f32 * 3.0 * area.height(),
                // Below: the two pictures share a column beside Settings.
                (Position::Below, _) => {
                    let pictures = usize::from(!folded[0]) + usize::from(!folded[1]);
                    let beside = pictures > 0 && !folded[2];
                    pictures.saturating_sub(1) as f32 * 3.0 * rects[0].width()
                        + if beside { 3.0 * area.height() } else { 0.0 }
                }
            };
            assert!(
                (covered + gaps - area.area()).abs() < 1.0,
                "{position:?} {folded:?}: {} pt² unaccounted for",
                area.area() - covered - gaps
            );
        }
    }
}

#[test]
fn a_narrow_window_takes_the_pictures_down_before_the_settings_floor() {
    let (rail, gap) = (24.0, 3.0);
    for position in [Position::Right, Position::Below] {
        let natural = Layout { position, ..Layout::default() }.natural_size(rail, gap, floor());
        // Two gaps between three columns on the right, one beside the pictures'
        // shared column below.
        let gaps = if position == Position::Right { 2.0 * gap } else { gap };
        // 300pt narrower would take the default 280pt column to about 195 if it
        // shrank in proportion; 60pt is too narrow for even the floor.
        for (width, settings) in [(natural.x - 300.0, floor()), (60.0, 60.0 - gaps)] {
            let mut layout = Layout { position, ..Layout::default() };
            let area = vec2(width, natural.y);
            layout.fit(area, rail, gap);
            let saved = layout.sizes();
            assert!(saved.settings < settings - 1.0, "fixture must cross the floor");
            let rects =
                layout.rects(Rect::from_min_size(egui::Pos2::ZERO, area), rail, gap, floor());
            let drawn = rects[Section::Settings as usize];
            assert!((drawn.width() - settings).abs() < 0.1, "{position:?} at {width}: {drawn:?}");
            assert!((drawn.right() - area.x).abs() < 0.1, "{position:?} at {width}: {drawn:?}");
            assert_eq!(layout.sizes(), saved, "drawing at the floor edited the saved sizes");
        }
    }
}
