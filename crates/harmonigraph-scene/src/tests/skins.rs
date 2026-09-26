//! The skin dials: every setting legible, and every background one grey.

use crate::skin::{contrast, oklab, Skin, SkinDials, LABEL_FLOOR, LIGHTNESS_RANGE, STEP};

/// Every combination of the dials' ends, the default between them, and the
/// hue circle every 30°. The ladder is linear in lightness and the checks
/// below move monotonically with tint and saturation, so a floor that holds
/// at the ends holds inside them; hue is not monotonic, so it is walked.
fn reachable() -> impl Iterator<Item = SkinDials> {
    let fresh = SkinDials::default();
    let lightnesses = [*LIGHTNESS_RANGE.start(), fresh.lightness, *LIGHTNESS_RANGE.end()];
    let fractions = [0.0, 0.25, 1.0];
    lightnesses.into_iter().flat_map(move |lightness| {
        (0..12).flat_map(move |step| {
            let hue = step as f32 * 30.0;
            fractions.into_iter().flat_map(move |tint| {
                fractions.into_iter().map(move |accent_saturation| SkinDials {
                    lightness,
                    tint_hue: hue,
                    tint,
                    // Offset, so a tint and an accent are not always one hue.
                    accent_hue: (hue + 150.0) % 360.0,
                    accent_saturation,
                })
            })
        })
    })
}

/// A floor every reachable skin clears, so a range that maps badly is caught
/// here rather than by squinting at the panel. Labels are secondary text on
/// the page; a bar's name sits on its fill where the fill runs under it; and
/// the fill has to read against its own empty track.
#[test]
fn every_skin_is_legible() {
    for dials in reachable() {
        let s = Skin::from_dials(dials);
        let label = contrast(s.text_dim, s.panel);
        let on_fill = contrast(s.text, s.accent_fill);
        let fill = contrast(s.accent_fill, s.well);
        assert!(label >= LABEL_FLOOR, "{dials:?}: labels {label:.2}:1 against the page");
        assert!(on_fill >= 3.0, "{dials:?}: bar names {on_fill:.2}:1 on the fill");
        assert!(fill >= 1.5, "{dials:?}: fill {fill:.2}:1 against its track");
    }
}

/// The neutral layers stand a whole number of steps above the page, in
/// order: a header one step up, tracks two, buttons three, hover four.
/// Within a byte's rounding of the asked lightness.
#[test]
fn the_neutral_layers_climb_one_step_at_a_time() {
    for dials in reachable() {
        let s = Skin::from_dials(dials);
        for (steps, what, colour) in [
            (0.0, "page", s.panel),
            (1.0, "header", s.header),
            (2.0, "track", s.well),
            (3.0, "button", s.widget),
            (4.0, "hover", s.widget_hover),
        ] {
            let want = dials.lightness + steps * STEP;
            let got = oklab(colour)[0];
            assert!(
                (got - want).abs() < 0.008,
                "{dials:?}: the {what} is at lightness {got:.3}, not {want:.3}",
            );
        }
    }
}

/// Backgrounds and text are greys at most slightly tinted, and all toward the
/// one hue; the accent does not leak into them.
#[test]
fn the_greys_share_one_slight_tint() {
    let dials =
        SkinDials { tint: 1.0, tint_hue: 60.0, accent_saturation: 1.0, ..Default::default() };
    let s = Skin::from_dials(dials);
    for colour in [s.panel, s.header, s.well, s.widget, s.widget_hover, s.text, s.text_dim] {
        let [_, a, b] = oklab(colour);
        let chroma = a.hypot(b);
        assert!(chroma <= crate::skin::TINT_MAX + 0.01, "{colour:?} has chroma {chroma:.3}");
        let hue = b.atan2(a).to_degrees().rem_euclid(360.0);
        assert!((hue - 60.0).abs() < 25.0, "{colour:?} leans toward {hue:.0}°, not 60°");
    }
    let untinted = Skin::from_dials(SkinDials { tint: 0.0, ..dials });
    let [_, a, b] = oklab(untinted.panel);
    assert!(a.hypot(b) < 0.005, "no tint is grey");
}

/// A hand-edited blob cannot put a dial outside its range or at NaN.
#[test]
fn saved_dials_are_brought_into_range() {
    let mut dials = SkinDials {
        lightness: 9.0,
        tint_hue: -40.0,
        tint: f32::NAN,
        accent_hue: 400.0,
        accent_saturation: 2.0,
    };
    dials.sanitize();
    let fresh = SkinDials::default();
    assert_eq!(
        dials,
        SkinDials {
            lightness: *LIGHTNESS_RANGE.end(),
            tint_hue: 0.0,
            tint: fresh.tint,
            accent_hue: 360.0,
            accent_saturation: 1.0,
        }
    );
}
