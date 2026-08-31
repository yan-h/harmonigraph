use crate::GlowCurve;

/// The editable point is on its global curve, and every stretch of that curve
/// descends. Those are separate claims: a formula can hit its parameter and
/// still turn back elsewhere in the domain.
#[test]
fn the_glow_curve_passes_through_its_point_without_turning_back() {
    for curve in [
        GlowCurve::default(),
        GlowCurve { distance: 0.84, level: 0.68 },
        GlowCurve { distance: 0.2, level: 0.8 },
        GlowCurve { distance: 0.9, level: 0.08 },
    ] {
        assert!(
            (curve.sample(curve.distance) - curve.level).abs() < 1e-6,
            "the curve missed its point: {curve:?}",
        );
        assert_eq!(curve.sample(0.0), 1.0, "the curve moved its full centre");
        assert_eq!(curve.sample(1.0), 0.0, "the curve moved its zero edge");
        let levels: Vec<f32> = (0..=400).map(|i| curve.sample(i as f32 / 400.0)).collect();
        assert!(
            levels.windows(2).all(|pair| pair[0] >= pair[1]),
            "the point made a curve that rises: {curve:?}",
        );
    }
}

/// The middle of the square is the ordinary straight falloff. This is the
/// anchor that makes movement above and below it read as bending one familiar
/// line rather than choosing two unrelated powers.
#[test]
fn the_middle_point_makes_a_linear_curve() {
    let curve = GlowCurve { distance: 0.5, level: 0.5 };
    assert_eq!(curve.exponents(), [1.0, 1.0]);
    for i in 0..=100 {
        let p = i as f32 / 100.0;
        assert!((curve.sample(p) - (1.0 - p)).abs() < 1e-6);
    }
}

/// Both point coordinates stop short of the fixed endpoints. At the endpoints
/// their logarithmic mapping has no finite, useful answer; the point still has
/// almost the whole square available to shape the curve.
#[test]
fn the_glow_curve_point_stays_inside_its_useful_domain() {
    let mut curve = GlowCurve::default();
    curve.set_point(-1.0, 2.0);
    assert_eq!(curve.point(), [0.06, 0.97]);
    curve.set_point(2.0, -1.0);
    assert_eq!(curve.point(), [0.94, 0.03]);
    assert!(curve.exponents().into_iter().all(f32::is_finite));
}

/// State outside the editor is repaired to the same finite rectangle the
/// point can make, including a non-finite value on either axis.
#[test]
fn a_glow_curve_from_state_is_repaired_before_it_reaches_the_picture() {
    let fresh = GlowCurve::default();
    let repaired = GlowCurve { distance: f32::NAN, level: f32::INFINITY }.sanitized();
    assert_eq!(repaired.point(), [fresh.distance, fresh.level]);
}
