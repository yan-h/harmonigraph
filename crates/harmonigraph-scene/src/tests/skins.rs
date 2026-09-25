//! The selectable skins: every one legible, and each saved under its own id.

use crate::skin::{contrast, oklab, skins, Chrome, DEFAULT_SKIN, LABEL_FLOOR};

/// Every [`Chrome`] the dials can reach: the four corners of the two ranges,
/// and the default between them. The ladder is linear in both, so a floor
/// that holds at the corners holds everywhere inside them.
fn reachable() -> Vec<Chrome> {
    let (l, s) = (Chrome::LIGHTNESS_RANGE, Chrome::STEP_RANGE);
    let mut all = vec![Chrome::default()];
    for lightness in [*l.start(), *l.end()] {
        for step in [*s.start(), *s.end()] {
            all.push(Chrome { lightness, step });
        }
    }
    all
}

/// A floor every skin clears at every reachable chrome, so a scheme or a
/// range that maps badly is caught here rather than by squinting at the
/// panel. Labels are secondary text on the page; a bar's name sits on its
/// fill where the fill runs under it; and the fill has to read against its
/// own empty track.
#[test]
fn every_skin_is_legible() {
    for entry in skins() {
        for chrome in reachable() {
            let s = entry.skin.stepped(chrome);
            let at = format!("{} at {chrome:?}", entry.id);
            let label = contrast(s.text_dim, s.panel);
            let on_fill = contrast(s.text, s.accent_fill);
            let fill = contrast(s.accent_fill, s.well);
            assert!(label >= LABEL_FLOOR, "{at}: labels {label:.2}:1 against the page");
            assert!(on_fill >= 3.0, "{at}: bar names {on_fill:.2}:1 on the fill");
            assert!(fill >= 1.5, "{at}: fill {fill:.2}:1 against its track");
        }
    }
}

/// The neutral layers stand a whole number of steps above the page, in
/// order, whatever the scheme: a header one step up, tracks two, buttons
/// three, hover four. Within a byte's rounding of the asked lightness.
#[test]
fn the_neutral_layers_climb_one_step_at_a_time() {
    for entry in skins() {
        for chrome in reachable() {
            let s = entry.skin.stepped(chrome);
            for (steps, what, colour) in [
                (0.0, "page", s.panel),
                (1.0, "header", s.header),
                (2.0, "track", s.well),
                (3.0, "button", s.widget),
                (4.0, "hover", s.widget_hover),
            ] {
                let want = chrome.lightness + steps * chrome.step;
                let got = oklab(colour)[0];
                assert!(
                    (got - want).abs() < 0.008,
                    "{} at {chrome:?}: the {what} is at lightness {got:.3}, not {want:.3}",
                    entry.id,
                );
            }
        }
    }
}

/// A hand-edited blob cannot put the dials outside their ranges or at NaN.
#[test]
fn chrome_sanitizes_into_its_ranges() {
    let wild = Chrome { lightness: 9.0, step: f32::NAN }.sanitize();
    assert_eq!(wild.lightness, *Chrome::LIGHTNESS_RANGE.end());
    assert_eq!(wild.step, Chrome::default().step);
}

#[test]
fn skins_have_distinct_ids_and_the_default_comes_first() {
    let ids: Vec<_> = skins().iter().map(|entry| entry.id).collect();
    assert_eq!(ids[0], DEFAULT_SKIN);
    let unique: std::collections::BTreeSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "{ids:?}");
}
