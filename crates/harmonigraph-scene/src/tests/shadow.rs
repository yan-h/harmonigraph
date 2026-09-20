//! The Shadow controls' scene contract.

use super::harness::*;
use crate::*;
use harmonigraph_core::{NoteTracker, Tuning};

/// The blob door and the picture door hold every Shadow GROUP to the same
/// endpoints, so a saved value never reads out differently from what is drawn.
///
/// Swept over the groups rather than named one at a time
/// ([`ShadowSettings::groups`]): a group added to the struct without a clamp is
/// invisible at its declaration, and this is what sees it.
#[test]
fn the_shadow_controls_keep_one_range_at_both_doors() {
    for (asked_width, widths, asked_depth, depth, asked_falloff, falloff) in [
        (-1.0, [0.0; 4], -0.5, 0.0, -9.0, SHADOW_FALLOFF_MIN),
        (2.0, [1.0, 1.0, 2.0, 2.0], 0.8, 0.8, 1.0, 1.0),
        (3.0, [1.0, 1.0, 3.0, 3.0], 1.0, 1.0, 1.0, 1.0),
        (9.0, [1.0, 1.0, 3.0, 3.0], 3.0, 1.0, 9.0, SHADOW_FALLOFF_MAX),
    ] {
        let asked = ShadowStyle {
            width: asked_width,
            depth: asked_depth,
            falloff: asked_falloff,
            ..ShadowStyle::default()
        };
        let want =
            widths.map(|width| ShadowStyle { width, depth, falloff, ..ShadowStyle::default() });
        let mut view = ViewConfig {
            shadow: ShadowSettings {
                lattice_geometry: asked,
                lattice_text: asked,
                spectral_geometry: asked,
                spectral_text: asked,
            },
            ..ViewConfig::default()
        };
        let scene =
            scene_of(&NoteTracker::new(), &Tuning::default(), &view, &FrameParams::default(), 0.0);
        for (group, drawn) in scene.shadow.groups().into_iter().enumerate() {
            assert_eq!(drawn, want[group], "the picture drew group {group} at {asked:?}");
        }

        view.sanitize();
        for (group, kept) in view.shadow.groups().into_iter().enumerate() {
            assert_eq!(kept, want[group], "the bar kept group {group} at {asked:?}");
        }
    }
}

#[test]
fn the_endpoint_has_exactly_four_explicit_shadow_groups() {
    let mut groups = ShadowSettings::default();
    for (index, group) in groups.groups_mut().into_iter().enumerate() {
        group.width = index as f32 * 0.1;
    }
    assert_eq!(
        groups.groups().map(|group| group.width),
        [0.0, 0.1, 0.2, 0.3],
        "a group is missing from the explicit endpoint or aliases another group",
    );
}

/// Either bar at its bottom is the whole of a group's shadow gone, and the two
/// are separate switches.
///
/// The claim the renderer's early-out rests on: `pack` reads this to decide
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

/// The full slider range has fixed endpoints, decreases monotonically, and
/// keeps one curvature sign. Include both sides of the near-linear series.
#[test]
fn falloff_runs_from_early_through_linear_to_late_without_inflections() {
    for falloff in
        (-60..=60).map(|i| i as f32 / 10.0).chain([-0.051, -0.049, -0.001, 0.001, 0.049, 0.051])
    {
        assert_eq!(standoff_level(falloff, 0.0), 1.0);
        assert_eq!(standoff_level(falloff, 1.0), 0.0);
        assert_eq!(standoff_level(falloff, 2.0), 0.0);
        let samples: Vec<_> = (0..=64).map(|i| standoff_level(falloff, i as f32 / 64.0)).collect();
        for pair in samples.windows(2) {
            assert!(pair[0] >= pair[1], "falloff {falloff} rises: {pair:?}");
        }
        for triple in samples.windows(3) {
            let curvature = triple[0] - 2.0 * triple[1] + triple[2];
            assert!(
                curvature * falloff.signum() <= 4.0e-7,
                "falloff {falloff} inflects: {triple:?}"
            );
        }
        for (i, &level) in samples.iter().enumerate() {
            let u = i as f32 / 64.0;
            let k = -f64::from(falloff);
            let exact = if k == 0.0 {
                1.0 - f64::from(u)
            } else {
                (k * (1.0 - f64::from(u))).exp_m1() / k.exp_m1()
            };
            assert!((f64::from(level) - exact).abs() < 2.0e-6);
            if falloff == 0.0 {
                assert_eq!(level, 1.0 - u);
            }
        }
    }
    assert!(standoff_level(SHADOW_FALLOFF_MIN, 0.5) < 0.05);
    assert!(standoff_level(SHADOW_FALLOFF_MAX, 0.5) > 0.95);
}
