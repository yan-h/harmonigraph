use crate::GlowCurve;

/// Every editable point is on the curve, and the stretches between them only
/// descend. Those are separate claims: an interpolant can hit every handle and
/// still overshoot between two of them.
#[test]
fn the_glow_curve_passes_through_its_handles_without_turning_back() {
    for curve in [
        GlowCurve::default(),
        GlowCurve { quarter: 0.9, half: 0.35, three_quarters: 0.3 },
        GlowCurve { quarter: 1.0, half: 1.0, three_quarters: 0.8 },
        GlowCurve { quarter: 0.2, half: 0.0, three_quarters: 0.0 },
    ] {
        let expected = [1.0, curve.quarter, curve.half, curve.three_quarters, 0.0];
        for (i, want) in expected.into_iter().enumerate() {
            assert_eq!(curve.sample(i as f32 / 4.0), want, "the curve missed handle {i}");
        }
        let levels: Vec<f32> = (0..=400).map(|i| curve.sample(i as f32 / 400.0)).collect();
        assert!(
            levels.windows(2).all(|pair| pair[0] >= pair[1]),
            "the descending handles made a curve that rises: {curve:?}",
        );
    }
}

/// A drag stops at its neighbour instead of swapping the meaning of two fixed
/// distances. The near, middle and far handles stay those three distances for
/// the whole gesture.
#[test]
fn one_glow_curve_handle_cannot_cross_another() {
    let mut curve = GlowCurve { quarter: 0.8, half: 0.5, three_quarters: 0.2 };
    curve.set_control(0, 0.1);
    assert_eq!(curve.controls(), [0.5, 0.5, 0.2]);
    curve.set_control(2, 0.9);
    assert_eq!(curve.controls(), [0.5, 0.5, 0.5]);
    curve.set_control(1, -1.0);
    assert_eq!(curve.controls(), [0.5, 0.5, 0.5]);

    let mut open = GlowCurve { quarter: 0.8, half: 0.5, three_quarters: 0.2 };
    open.set_control(1, -1.0);
    assert_eq!(open.controls(), [0.8, 0.2, 0.2]);
}

/// State outside the editor is repaired to the same finite descending shape
/// its handles can make, including a non-finite value at every position.
#[test]
fn a_glow_curve_from_state_is_repaired_before_it_reaches_the_picture() {
    let fresh = GlowCurve::default();
    let repaired =
        GlowCurve { quarter: f32::NAN, half: 2.0, three_quarters: f32::INFINITY }.sanitized();
    assert_eq!(repaired.controls(), [fresh.quarter, fresh.quarter, fresh.three_quarters],);
    assert!(
        repaired.controls().windows(2).all(|pair| pair[0] >= pair[1]),
        "the repaired curve still rises: {repaired:?}",
    );
}
