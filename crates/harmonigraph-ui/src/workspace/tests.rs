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
