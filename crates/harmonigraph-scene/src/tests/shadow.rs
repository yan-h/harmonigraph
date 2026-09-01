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

/// Softness partitions one fixed Shadow reach between exact dilation and a
/// three-sigma Gaussian tail. The kernel table works in picture σ, which is
/// half the displayed reference width, so every setting must still end at the
/// row's calibrated 2.5 table σ.
#[test]
fn softness_divides_one_fixed_shadow_reach() {
    let fresh = ViewConfig::default();
    assert_eq!(fresh.glow_shadow_kernel, ShadowKernel::Spread);
    assert_eq!(fresh.glow_shadow, 0.16);
    assert_eq!(fresh.glow_shadow_softness, GLOW_SHADOW_SOFTNESS_DEFAULT);
    assert_eq!(
        ShadowKernel::Spread.terms_with(fresh.glow_shadow_softness),
        ShadowKernel::Spread.terms(),
        "the adjustable row must reproduce its calibrated defaults",
    );
    let non_finite = ShadowKernel::Spread.terms_with(f32::NAN);
    assert_eq!(non_finite[0].spread, 2.5);
    assert_eq!(non_finite[0].sigma, 0.0);

    for (asked, softness) in [(-1.0, 0.0), (0.35, 0.35), (2.0, 1.0)] {
        let mut view = ViewConfig {
            glow_shadow_softness: asked,
            glow_shadow_kernel: ShadowKernel::Spread,
            ..ViewConfig::default()
        };
        let scene =
            scene_of(&NoteTracker::new(), &Tuning::default(), &view, &FrameParams::default(), 0.0);
        assert_eq!(scene.glow_shadow_softness, softness);

        let terms = ShadowKernel::Spread.terms_with(scene.glow_shadow_softness);
        assert!((terms[0].spread - 2.5 * (1.0 - softness)).abs() < 1e-6);
        assert!((terms[0].sigma - 2.5 * softness / REACH_SIGMAS).abs() < 1e-6);
        assert!((terms[0].reach_sigmas() - 2.5).abs() < 1e-6);

        view.sanitize();
        assert_eq!(view.glow_shadow_softness, softness);
    }
}
