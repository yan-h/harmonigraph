//! A [`Bend`]'s curve, and what it may and may not move in a gradient.

use crate::{Bend, Gradient};

/// Corners across the whole plot, the straight ones and both extremes of
/// where one can stand among them.
fn bends() -> impl Iterator<Item = Bend> {
    let ats = (0..=24).map(|i| 0.02 + 0.04 * i as f32);
    ats.flat_map(|at| (0..=20).map(move |j| Bend { at, share: j as f32 / 20.0, ..Bend::default() }))
}

#[test]
fn a_bend_rises_smoothly_from_line_to_line_with_both_ends_pinned() {
    for b in bends() {
        assert_eq!(b.warp(0.0), 0.0, "{b:?} moved the bottom end");
        assert_eq!(b.warp(1.0), 1.0, "{b:?} moved the top end");
        let (x, y) = (f64::from(b.at), f64::from(b.share));
        // Short of the round, the curve IS the line into the corner.
        let before = x * (1.0 - Bend::ROUNDING) * 0.5;
        assert!((b.warp(before) - y / x * before).abs() < 1e-9, "{b:?} left its first line");
        let (n, mut last) = (4000, 0.0);
        for i in 1..=n {
            let t = f64::from(i) / f64::from(n);
            let w = b.warp(t);
            assert!(w >= last, "{b:?} runs backwards at t = {t}: {w} after {last}");
            if b.is_straight() {
                assert_eq!(w, t, "a straight bend is not the identity at t = {t}");
            }
            last = w;
        }
        // No corner: where the round meets each line it leaves along it, so
        // the slope just inside matches the slope just outside. Between those
        // points the round is one quadratic Bézier, smooth by construction.
        if b.is_straight() {
            continue;
        }
        // Probed a small share of the round's own width either side, since
        // at a corner near the edge one side of the round is under 1% wide.
        let r = Bend::ROUNDING;
        let near = 1e-4 * r * x.min(1.0 - x);
        let slope = |t: f64| (b.warp(t + near * 0.01) - b.warp(t - near * 0.01)) / (near * 0.02);
        for (join, line) in [(x * (1.0 - r), y / x), (x + r * (1.0 - x), (1.0 - y) / (1.0 - x))] {
            for side in [-near, near] {
                let got = slope(join + side);
                assert!(
                    (got - line).abs() < 1e-2 * line.max(1.0),
                    "{b:?} meets its line at {join} with slope {got}, not {line}",
                );
            }
        }
    }
}

#[test]
fn a_bend_moves_only_the_channels_on_it_and_never_their_ends() {
    // With a chroma ramp, which the default lacks, so the curve has one to shape.
    let even = Gradient { chroma_ramp: 0.4, ..Gradient::default() };
    let bend = Bend { at: 0.9, share: 0.15, hue: true, lightness: false, chroma: true };
    let bent = Gradient { bend, ..even };
    let color = |t: f64, g: Gradient| crate::color::designed_pitch_ramp(t, g);
    for t in [0.0, 1.0] {
        assert_eq!(color(t, bent), color(t, even), "a bend moved the end at t = {t}");
    }
    // The case the bend is for: halfway up the range the hue has walked a
    // twelfth of the way the even arc would have.
    let hue = |t: f64, g: Gradient| g.lightness_and_hue(t).1;
    let walked = (hue(0.5, bent) - hue(0.0, bent)).rem_euclid(360.0);
    let want = 0.5 * 0.15 / 0.9 * f64::from(even.hue_span);
    assert!((walked - want).abs() < 1e-3, "walked {walked}, not {want}");
    assert!((bent.chroma_at(0.5) - even.chroma_at(0.5)).abs() > 1e-3, "chroma ignored the bend");
    // Brightness is off the curve, so it walks exactly as before.
    for i in 0..=10 {
        let t = f64::from(i) / 10.0;
        assert_eq!(bent.lightness_and_hue(t).0, even.lightness_and_hue(t).0);
    }
}
