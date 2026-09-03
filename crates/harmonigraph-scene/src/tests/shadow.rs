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
    for (asked_width, width, asked_depth, depth) in [(-1.0, 0.0, -0.5, 0.0), (2.0, 1.0, 3.0, 1.0)] {
        let asked =
            ShadowStyle { width: asked_width, depth: asked_depth, ..ShadowStyle::default() };
        let want = ShadowStyle { width, depth, ..ShadowStyle::default() };
        let mut view = ViewConfig {
            shadow: ShadowSettings { lattice_geometry: asked, lattice_text: asked },
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
