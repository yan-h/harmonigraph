//! The selectable skins: every one legible, and each saved under its own id.

use crate::skin::{contrast, skins, DEFAULT_SKIN, PICTURE};

/// A floor every skin clears, so one added from a scheme that maps badly is
/// caught here rather than by squinting at the panel. Labels are secondary
/// text on the pane; a bar's name sits on its fill where the fill runs under
/// it; and the fill has to read against its own empty track. How visible the
/// empty track is against the pane is left to taste: the chosen schemes run
/// from 1.06 to 1.33 there, and the original's is 1.08.
#[test]
fn every_skin_is_legible() {
    for entry in skins() {
        let s = &entry.skin;
        let label = contrast(s.text_dim, s.panel);
        let on_fill = contrast(s.text, s.accent_fill);
        let fill = contrast(s.accent_fill, s.well);
        assert!(label >= 4.0, "{}: labels {label:.2}:1 against the pane", entry.id);
        assert!(on_fill >= 3.0, "{}: bar names {on_fill:.2}:1 on the fill", entry.id);
        assert!(fill >= 1.5, "{}: fill {fill:.2}:1 against its track", entry.id);
    }
}

/// The pane header stands apart from the page below it and from the black
/// picture ground, and the fold button's square still stands apart from it.
#[test]
fn every_skin_has_a_header_between_its_pane_and_its_buttons() {
    for entry in skins() {
        let s = &entry.skin;
        let header = s.header();
        for (what, against) in [("pane", s.panel), ("button", s.widget), ("picture", PICTURE)] {
            let ratio = contrast(header, against);
            assert!(ratio >= 1.04, "{}: header {ratio:.2}:1 against the {what}", entry.id);
        }
    }
}

#[test]
fn skins_have_distinct_ids_and_the_default_comes_first() {
    let ids: Vec<_> = skins().iter().map(|entry| entry.id).collect();
    assert_eq!(ids[0], DEFAULT_SKIN);
    let unique: std::collections::BTreeSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "{ids:?}");
}
