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
        (-1.0, [0.0; 4], -0.5, 0.0, 0.0, SHADOW_FALLOFF_MIN),
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

/// One Shadow width out, a distance shadow stands at the same level whatever
/// the falloff is dialled to — so the Shadow width bar keeps its meaning across
/// the whole of the falloff bar, and the two are free of each other.
///
/// The property the whole design rests on, and what makes this exponent worth
/// having where the `glow_shadow_shape` that #563 removed was not: `pow(u, f)`
/// fixes `u = 1` because `1^f` is 1, where an exponent on the finished COVERAGE
/// gives `exp(-TAIL · f)` there and so is the width bar wearing another name. A
/// falloff that moved this number would be one the Shadow bar had to be
/// re-dialled after.
#[test]
fn one_shadow_width_holds_the_same_level_at_every_falloff() {
    let whole = (-SHADOW_TAIL).exp();
    for step in 0..=8 {
        let falloff =
            SHADOW_FALLOFF_MIN + (SHADOW_FALLOFF_MAX - SHADOW_FALLOFF_MIN) * step as f32 / 8.0;
        let level = standoff_level(falloff, 1.0);
        assert!(
            (level - whole).abs() < 1.0e-6,
            "at falloff {falloff} one width out reads {level} against the {whole} every other \
             falloff reads, so the width bar means something different here",
        );
    }
    // And the bar has its whole travel somewhere: half a width in, which is the
    // part of the profile it bends.
    let (sharp, plateau) =
        (standoff_level(SHADOW_FALLOFF_MIN, 0.5), standoff_level(SHADOW_FALLOFF_MAX, 0.5));
    assert!(
        plateau > 4.0 * sharp,
        "across the whole bar half a width out moves {sharp} to {plateau}, less than the factor \
         of four that would make it a bar with something on it",
    );
}

/// Every falloff on the bar shuts its window on a shadow the eye cannot see,
/// because the CELL follows the exponent rather than the exponent being held to
/// the cell.
///
/// The claim that lets the bar reach 0.35 at all. A falloff under 1 carries the
/// decay further out; the predecessor bar left the pad fixed and cut a tail
/// still standing at 0.9%, drawing the closed contour at a fixed radius that
/// the whole family exists to avoid. Here [`shadow_stop`] solves for the radius
/// instead, so what a low falloff costs is atlas rather than a ring.
#[test]
fn every_falloff_shuts_its_window_on_nothing() {
    for step in 0..=12 {
        let falloff =
            SHADOW_FALLOFF_MIN + (SHADOW_FALLOFF_MAX - SHADOW_FALLOFF_MIN) * step as f32 / 12.0;
        let stop = shadow_stop(falloff);
        // The raw decay where this falloff's cell stops, against the threshold
        // the stop was solved for.
        let raw = (-SHADOW_TAIL * stop.powf(falloff)).exp();
        assert!(
            raw <= SHADOW_INVISIBLE * 1.01,
            "at falloff {falloff} the decay is still {raw} at its own stop of {stop}, against \
             the {SHADOW_INVISIBLE} that is invisible",
        );
        // And the window really does end there, exactly.
        assert_eq!(
            standoff_level(falloff, stop),
            0.0,
            "at falloff {falloff} the shadow is still standing where its cell stops",
        );
    }
    // The crossover is where the solve meets the floor, and it is a rounded
    // literal because `ln` is not a `const fn` — so it is held to its own
    // algebra here rather than trusted. Both spellings of `shadow_stop` BRANCH
    // on it, so a drift would put a step in the stop exactly where the two
    // expressions are supposed to agree.
    let solved = ((1.0f32 / SHADOW_INVISIBLE).ln() / SHADOW_TAIL).powf(1.0 / SHADOW_FALLOFF_FREE);
    assert!(
        (solved - SHADOW_STOP).abs() < 1.0e-4,
        "at the crossover {SHADOW_FALLOFF_FREE} the solve reaches {solved}, not the \
         {SHADOW_STOP} it is supposed to meet",
    );
    // The fixed stop is a FLOOR and holds wherever it is already generous, so
    // a fresh picture is padded and windowed exactly as it was before the bar
    // existed. Everything from the crossover up pays nothing.
    for falloff in [SHADOW_FALLOFF_FREE, 0.65, 0.8, 1.0, 2.0, SHADOW_FALLOFF_MAX] {
        assert_eq!(
            shadow_stop(falloff),
            SHADOW_STOP,
            "falloff {falloff} moved the stop off its floor, so it pads cells for nothing",
        );
    }
    // And below the crossover it grows, or the bottom of the bar is the old
    // cut-off tail wearing a new number.
    assert!(
        shadow_stop(SHADOW_FALLOFF_MIN) > 1.7 * SHADOW_STOP,
        "the bottom of the bar pads to {} against the fixed {SHADOW_STOP}, which is not a cell \
         following its exponent",
        shadow_stop(SHADOW_FALLOFF_MIN),
    );
}
