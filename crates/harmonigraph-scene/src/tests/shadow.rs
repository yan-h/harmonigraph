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

/// Spread and Blur are independent shares of the displayed Shadow width. The
/// kernel table works in picture σ, which is half that width, so this pins the
/// conversion as well as the saved-view and derived-scene ranges.
#[test]
fn spread_and_blur_are_independent_width_controls() {
    let fresh = ViewConfig::default();
    assert_eq!(fresh.glow_shadow_spread, 0.5);
    assert_eq!(fresh.glow_shadow_blur, 0.25);
    assert_eq!(
        ShadowKernel::Spread.terms_with(fresh.glow_shadow_spread, fresh.glow_shadow_blur),
        ShadowKernel::Spread.terms(),
        "the adjustable row must reproduce its calibrated defaults",
    );

    for (asked_spread, spread, asked_blur, blur) in
        [(-1.0, 0.0, 0.40, 0.40), (0.20, 0.20, 2.0, 1.0)]
    {
        let mut view = ViewConfig {
            glow_shadow_spread: asked_spread,
            glow_shadow_blur: asked_blur,
            glow_shadow_kernel: ShadowKernel::Spread,
            ..ViewConfig::default()
        };
        let scene =
            scene_of(&NoteTracker::new(), &Tuning::default(), &view, &FrameParams::default(), 0.0);
        assert_eq!(scene.glow_shadow_spread, spread);
        assert_eq!(scene.glow_shadow_blur, blur);

        let terms =
            ShadowKernel::Spread.terms_with(scene.glow_shadow_spread, scene.glow_shadow_blur);
        assert_eq!(terms[0].spread, 2.0 * spread, "Spread was not converted from whole widths");
        assert_eq!(terms[0].sigma, 2.0 * blur, "Blur was not converted from whole widths");
        assert_eq!(terms[0].reach_sigmas(), 2.0 * spread + 6.0 * blur);

        view.sanitize();
        assert_eq!(view.glow_shadow_spread, spread);
        assert_eq!(view.glow_shadow_blur, blur);
    }
}
