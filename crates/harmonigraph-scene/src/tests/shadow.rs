//! The Shadow controls' scene contract.

use super::harness::*;
use crate::*;

/// A non-finite Shadow field repairs to its own group's fresh value, as the
/// recorded rule for nested groups has it (#1677 area 5 Q2): every field of
/// every group, since the text groups' fresh Spread is not the 0 a bare
/// clamp falls back to (#1847).
#[test]
fn a_non_finite_shadow_field_repairs_to_its_groups_fresh_value() {
    let fresh = ViewConfig::default().shadow;
    let groups = fresh.groups();
    for (name, values) in [
        ("width", groups.map(|g| g.width)),
        ("spread", groups.map(|g| g.spread)),
        ("depth", groups.map(|g| g.depth)),
    ] {
        assert!(
            values.iter().any(|&value| value != 0.0),
            "no fresh group holds its {name} off 0, so a repair to 0 would pass",
        );
    }
    for poison in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut view = ViewConfig::default();
        for group in view.shadow.groups_mut() {
            *group = ShadowStyle { width: poison, spread: poison, depth: poison };
        }
        view.sanitize();
        assert_eq!(view.shadow.groups(), fresh.groups(), "{poison} did not repair to fresh");
    }
}

/// The load holds every Shadow GROUP to its bar's endpoints, so a saved value
/// never reads out differently from what the bar can set.
///
/// Swept over the groups rather than named one at a time
/// ([`ShadowSettings::groups`]): a group added to the struct without a clamp is
/// invisible at its declaration, and this is what sees it.
#[test]
fn the_shadow_controls_load_inside_their_bars() {
    for (asked_width, widths, asked_spread, spread, asked_depth, depth) in [
        (-1.0, [0.0; 3], -0.5, 0.0, -0.5, 0.0),
        (2.0, [1.0, 1.0, 2.0], 0.17, 0.17, 0.8, 0.8),
        (3.0, [1.0, 1.0, 3.0], 1.0, 1.0, 1.0, 1.0),
        (9.0, [1.0, 1.0, 3.0], 3.0, 1.0, 3.0, 1.0),
    ] {
        let asked = ShadowStyle { width: asked_width, spread: asked_spread, depth: asked_depth };
        let want = widths.map(|width| ShadowStyle { width, spread, depth });
        let view = ViewConfig {
            shadow: ShadowSettings {
                lattice_geometry: asked,
                lattice_text: asked,
                spectral_text: asked,
            },
            ..ViewConfig::default()
        };
        for (group, kept) in loaded(&view).shadow.groups().into_iter().enumerate() {
            assert_eq!(kept, want[group], "the load kept group {group} at {asked:?}");
        }
    }
}

#[test]
fn gaussian_spread_uses_resolved_width() {
    let style = ShadowStyle { spread: 0.17, ..Default::default() };
    assert!((style.gaussian_spread_points(6.0) - 2.04).abs() < 1e-6);
    assert_eq!(ShadowStyle { spread: f32::NAN, ..style }.gaussian_spread_points(6.0), 0.0);
    assert_eq!(ShadowStyle { spread: 2.0, ..style }.gaussian_spread_points(6.0), 12.0);
}

#[test]
fn text_spread_is_stops_held_to_the_bar() {
    let style = ShadowStyle { spread: 0.4, ..Default::default() };
    assert_eq!(style.text_spread_stops(), 2.0);
    assert_eq!(ShadowStyle { spread: f32::NAN, ..style }.text_spread_stops(), 0.0);
    assert_eq!(ShadowStyle { spread: 2.0, ..style }.text_spread_stops(), TEXT_SPREAD_STOPS);
}

#[test]
fn the_endpoint_has_exactly_three_explicit_shadow_groups() {
    let mut groups = ShadowSettings::default();
    for (index, group) in groups.groups_mut().into_iter().enumerate() {
        group.width = index as f32 * 0.1;
    }
    assert_eq!(
        groups.groups().map(|group| group.width),
        [0.0, 0.1, 0.2],
        "a group is missing from the explicit endpoint or aliases another group",
    );
}

/// Either bar at its bottom is the whole of a group's shadow gone, and the two
/// are separate switches.
///
/// The claim the renderer's early-out rests on: `pack_into` reads this to decide
/// whether a caster takes a cell at all, so a group that reads as casting with
/// one bar shut would allocate atlas and rasterize ink for a shadow the picture
/// never spends.
#[test]
fn a_group_with_either_bar_at_its_bottom_casts_nothing() {
    let fresh = ShadowStyle::default();
    assert!(fresh.casts(), "the fresh style casts nothing");
    assert!(!ShadowStyle { width: 0.0, ..fresh }.casts(), "a group at no width still casts");
    assert!(!ShadowStyle { depth: 0.0, ..fresh }.casts(), "a group at no depth still casts");
}
