//! A [`Bend`]'s curve, and what it may and may not move in a gradient.

use crate::{Bend, Gradient};

/// Every bend the grid below reaches, the straight one and both extremes of
/// where it can stand among them.
fn bends() -> impl Iterator<Item = Bend> {
    let ats = (0..=20).map(|i| (0.01 + 0.049 * i as f32).min(Bend::AT_LIMITS.1));
    ats.flat_map(|at| (0..=20).map(move |j| Bend { at, share: j as f32 / 20.0 }))
}

#[test]
fn a_bend_is_monotone_through_its_point_and_pinned_at_both_ends() {
    for b in bends() {
        assert_eq!(b.warp(0.0), 0.0, "{b:?} moved the bottom end");
        assert_eq!(b.warp(1.0), 1.0, "{b:?} moved the top end");
        let through = b.warp(f64::from(b.at));
        assert!((through - f64::from(b.share)).abs() < 1e-6, "{b:?} missed its point: {through}");
        let mirror = b.mirrored();
        let mut last = 0.0;
        for i in 0..=2000 {
            let t = f64::from(i) / 2000.0;
            let w = b.warp(t);
            assert!(w >= last, "{b:?} runs backwards at t = {t}: {w} after {last}");
            last = w;
            if b.is_straight() {
                assert_eq!(w, t, "a straight bend is not the identity at t = {t}");
            }
            // Not exact: `1 - at` rounds in f32, and a bend at the edge of
            // the range is steep enough to show that in the fifth place.
            let read_back = 1.0 - mirror.warp(1.0 - t);
            assert!((read_back - w).abs() < 1e-5, "{b:?} mirrored is not the same curve reversed");
        }
    }
}

#[test]
fn a_bend_moves_the_middle_of_a_gradient_and_never_its_ends() {
    let even = Gradient::default();
    let bend = Bend { at: 0.9, share: 0.15 };
    let bent = Gradient { hue_bend: bend, lightness_bend: bend, chroma_bend: bend, ..even };
    let color = |t: f64, g: Gradient| crate::color::designed_pitch_ramp(t, g);
    for t in [0.0, 1.0] {
        assert_eq!(color(t, bent), color(t, even), "a bend moved the end at t = {t}");
    }
    // The case the bend is for: most of the range barely moves, the top turns.
    let hue = |t: f64, g: Gradient| g.lightness_and_hue(t).1;
    let walked = |t: f64| (hue(t, bent) - hue(0.0, bent)).rem_euclid(360.0);
    assert!((walked(0.9) - 0.15 * f64::from(even.hue_span)).abs() < 1e-3);

    // A flipped arc reads the SAME walk backwards, bend and all, rather than a
    // different curve over the same colors.
    let flipped = bent.flipped();
    for i in 0..=100 {
        let t = f64::from(i) / 100.0;
        let (h, back) = (hue(t, bent), hue(1.0 - t, flipped));
        let apart = (h - back).rem_euclid(360.0);
        assert!(apart.min(360.0 - apart) < 1e-3, "flip read t = {t} as {back}, not {h}");
    }
}
