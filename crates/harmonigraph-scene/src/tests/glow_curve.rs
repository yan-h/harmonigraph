use crate::GlowCurve;

/// Every shape descends between the same fixed endpoints.
#[test]
fn the_glow_curve_descends_between_its_fixed_endpoints() {
    for curve in [
        GlowCurve::default(),
        GlowCurve { shape: -8.0 },
        GlowCurve { shape: 0.0 },
        GlowCurve { shape: 8.0 },
    ] {
        assert_eq!(curve.sample(0.0), 1.0, "the curve moved its full centre");
        assert_eq!(curve.sample(1.0), 0.0, "the curve moved its zero edge");
        let levels: Vec<f32> = (0..=400).map(|i| curve.sample(i as f32 / 400.0)).collect();
        assert!(
            levels.windows(2).all(|pair| pair[0] >= pair[1]),
            "the point made a curve that rises: {curve:?}",
        );
    }
}

/// Zero is the ordinary straight falloff, the centre of the signed slider.
#[test]
fn zero_shape_makes_a_linear_curve() {
    let curve = GlowCurve { shape: 0.0 };
    assert_eq!(curve.shape(), 0.0);
    for i in 0..=100 {
        let p = i as f32 / 100.0;
        assert!((curve.sample(p) - (1.0 - p)).abs() < 1e-6);
    }
}

/// A positive shape produces one convex exponential-like decay, not a curve
/// that flattens at both ends and turns through an S between them.
#[test]
fn a_positive_shape_falls_quickly_without_an_inflection() {
    let curve = GlowCurve { shape: 4.0 };
    assert!(curve.sample(0.7) < 0.05, "the curve still carries too much light into its tail");
    let levels: Vec<f32> = (0..=200).map(|i| curve.sample(i as f32 / 200.0)).collect();
    let drops: Vec<f32> = levels.windows(2).map(|pair| pair[0] - pair[1]).collect();
    assert!(
        drops.windows(2).all(|pair| pair[0] + 1e-6 >= pair[1]),
        "the fast curve changes from convex to concave",
    );
}

/// State outside the slider is repaired to the slider's finite range.
#[test]
fn a_glow_curve_from_state_is_repaired_before_it_reaches_the_picture() {
    let fresh = GlowCurve::default();
    assert_eq!(GlowCurve { shape: f32::NAN }.sanitized(), fresh);
    assert_eq!(GlowCurve { shape: f32::INFINITY }.sanitized(), fresh);
    assert_eq!(GlowCurve { shape: f32::NEG_INFINITY }.sanitized(), fresh);
    assert_eq!(GlowCurve { shape: 20.0 }.sanitized().shape, 8.0);
    assert_eq!(GlowCurve { shape: -20.0 }.sanitized().shape, -8.0);
}
