//! The Shadow controls' scene contract.

use super::harness::*;
use crate::*;
use harmonigraph_core::{NoteTracker, Tuning};

/// The blob door and the picture door hold Shadow width and curve to the same
/// endpoints, so a saved value never reads out differently from what is drawn.
#[test]
fn the_shadow_controls_keep_one_range_at_both_doors() {
    for (asked_shadow, shadow, asked_curve, curve) in [(-1.0, 0.0, 0.25, 1.0), (2.0, 1.0, 5.0, 4.0)]
    {
        let mut view = ViewConfig {
            glow_shadow: asked_shadow,
            glow_shadow_curve: asked_curve,
            ..ViewConfig::default()
        };
        let scene =
            scene_of(&NoteTracker::new(), &Tuning::default(), &view, &FrameParams::default(), 0.0);
        assert_eq!(scene.glow_shadow, shadow, "the picture drew a Shadow of {asked_shadow}");
        assert_eq!(
            scene.glow_shadow_curve, curve,
            "the picture drew a Shadow curve of {asked_curve}",
        );

        view.sanitize();
        assert_eq!(view.glow_shadow, shadow, "the bar kept a Shadow of {asked_shadow}");
        assert_eq!(view.glow_shadow_curve, curve, "the bar kept a Shadow curve of {asked_curve}");
    }
}
