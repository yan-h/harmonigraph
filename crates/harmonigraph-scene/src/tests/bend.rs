//! A [`Bend`]'s curve, and what it may and may not move in a gradient.

use crate::{Bend, Gradient};

#[test]
fn a_bend_rises_through_its_knee_and_is_pinned_at_both_ends() {
    for i in 0..=96 {
        let knee = 0.02 + 0.01 * i as f32;
        let b = Bend { knee, ..Bend::default() };
        assert_eq!(b.warp(0.0), 0.0, "{b:?} moved the bottom end");
        assert_eq!(b.warp(1.0), 1.0, "{b:?} moved the top end");
        let through = b.warp(f64::from(knee));
        assert!((through - (1.0 - f64::from(knee))).abs() < 1e-6, "{b:?} missed its knee");
        let mut last = 0.0;
        for j in 0..=2000 {
            let t = f64::from(j) / 2000.0;
            let w = b.warp(t);
            assert!(w >= last, "{b:?} runs backwards at t = {t}: {w} after {last}");
            last = w;
            if b.is_straight() {
                assert_eq!(w, t, "a straight bend is not the identity at t = {t}");
            }
        }
    }
}

#[test]
fn a_bend_moves_only_the_channels_on_it_and_never_their_ends() {
    // With a chroma ramp, which the default lacks, so the curve has one to shape.
    let even = Gradient { chroma_ramp: 0.4, ..Gradient::default() };
    let bend = Bend { knee: 0.9, hue: true, lightness: false, chroma: true };
    let bent = Gradient { bend, ..even };
    let color = |t: f64, g: Gradient| crate::color::designed_pitch_ramp(t, g);
    for t in [0.0, 1.0] {
        assert_eq!(color(t, bent), color(t, even), "a bend moved the end at t = {t}");
    }
    // The case the bend is for: most of the range barely moves, the top turns.
    let hue = |t: f64, g: Gradient| g.lightness_and_hue(t).1;
    let walked = (hue(0.9, bent) - hue(0.0, bent)).rem_euclid(360.0);
    assert!((walked - 0.1 * f64::from(even.hue_span)).abs() < 1e-3, "walked {walked}");
    assert!((bent.chroma_at(0.5) - even.chroma_at(0.5)).abs() > 1e-3, "chroma ignored the bend");
    // Brightness is off the curve, so it walks exactly as before.
    for i in 0..=10 {
        let t = f64::from(i) / 10.0;
        assert_eq!(bent.lightness_and_hue(t).0, even.lightness_and_hue(t).0);
    }
}
