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
    for (asked_width, width, asked_depth, depth, asked_falloff, falloff) in [
        (-1.0, 0.0, -0.5, 0.0, 0.0, SHADOW_FALLOFF_MIN),
        (2.0, 1.0, 3.0, 1.0, 9.0, SHADOW_FALLOFF_MAX),
    ] {
        let asked = ShadowStyle {
            width: asked_width,
            depth: asked_depth,
            falloff: asked_falloff,
            ..ShadowStyle::default()
        };
        let want = ShadowStyle { width, depth, falloff, ..ShadowStyle::default() };
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
            assert_eq!(drawn, want, "the picture drew group {group} at {asked:?}");
        }

        view.sanitize();
        for (group, kept) in view.shadow.groups().into_iter().enumerate() {
            assert_eq!(kept, want, "the bar kept group {group} at {asked:?}");
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

/// The bottom of the falloff bar still lands under the window on a shadow the
/// eye cannot see, so no setting of it draws the closed contour at a fixed
/// radius that [`SHADOW_STOP`] exists to prevent.
///
/// What SETS [`SHADOW_FALLOFF_MIN`] rather than a check on it: a falloff under
/// 1 carries the decay further out, and the stop is a CELL's padding — it
/// cannot widen to follow. The predecessor bar took this trade the other way
/// and cut a tail still standing at 0.9%.
#[test]
fn the_falloff_bar_cannot_reopen_the_window() {
    // Half a code value of the deepest shadow: the threshold `SHADOW_STOP`'s
    // own doc is written against.
    let invisible = 0.5 / 255.0;
    let raw = |falloff: f32| (-SHADOW_TAIL * SHADOW_STOP.powf(falloff)).exp();
    assert!(
        raw(SHADOW_FALLOFF_MIN) < invisible,
        "at the bottom of the bar the decay is {} where the window shuts, against the {} that is \
         invisible — so the window would cut a contour the eye can see",
        raw(SHADOW_FALLOFF_MIN),
        invisible,
    );
    // And it stops close to that edge rather than well inside it, so the sharp
    // half of the bar is as long as the window allows it to be.
    assert!(
        raw(SHADOW_FALLOFF_MIN - 0.1) > 0.5 * invisible,
        "the bar could start below {SHADOW_FALLOFF_MIN} and still shut on nothing",
    );
    // The window itself, which is what the picture spends: exactly zero at the
    // stop whatever the falloff, because it is measured in widths and the
    // exponent does not reach it.
    for step in 0..=4 {
        let falloff =
            SHADOW_FALLOFF_MIN + (SHADOW_FALLOFF_MAX - SHADOW_FALLOFF_MIN) * step as f32 / 4.0;
        assert_eq!(
            standoff_level(falloff, SHADOW_STOP),
            0.0,
            "at falloff {falloff} the shadow is still standing where its cell stops",
        );
    }
}
