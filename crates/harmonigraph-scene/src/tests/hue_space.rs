//! The two spaces the ramp is authored in: checks that the chroma knob leaves
//! the hue alone and that the join keeps `L*`'s promise at the edges.
//!
//! The measurements behind #222's decision to move the ramp onto Oklab, and
//! the pricing of a move on to CAM16, were printing probes here until #1181;
//! the issues hold their verdicts, and the history holds the probes.

use crate::style::Gradient;
use glam::Vec4;

/// Oklch of a color that is already sRGB, which is what the LUT holds.
///
/// Measuring the DRAWN color rather than converting Lab coordinates across is
/// what keeps this honest about white points: whatever `color_space`'s Lab
/// assumes, the pixel is the pixel, and Oklab's matrices are defined against
/// sRGB's own primaries.
fn oklch(c: Vec4) -> (f64, f64, f64) {
    let lin = |v: f32| {
        let v = f64::from(v);
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (lin(c.x), lin(c.y), lin(c.z));
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    let ok_l = 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s;
    let ok_a = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s;
    let ok_b = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s;
    (ok_l, ok_a.hypot(ok_b), ok_b.atan2(ok_a).to_degrees().rem_euclid(360.0))
}

/// Signed shortest way round from `a` to `b`, in degrees.
fn hue_delta(a: f64, b: f64) -> f64 {
    (b - a + 540.0).rem_euclid(360.0) - 180.0
}

/// The eleven deciles of `t`.
fn deciles() -> impl Iterator<Item = f64> {
    (0..=10).map(|k| f64::from(k) / 10.0)
}

/// The CIELAB arc the shipped defaults are a conversion OF, in CIELAB degrees.
/// Named once, beside the test that holds the defaults to it.
const RETIRED_ARC_START: f64 = 260.0;
const RETIRED_ARC_SPAN: f64 = 190.0;

/// The chroma fraction that arc was DRAWN at, which is part of the historical
/// object and not a reading of today's gradient.
///
/// A CIELAB hue angle is not one hue — its color swings up to 10 degrees toward
/// purple between a fifth of the gamut and all of it — so which color the
/// retired arc opened on depends on how much chroma it opened with, and that
/// was half of what CIELAB held. Reading `Gradient::chroma` here instead would
/// make the retired arc move whenever the fresh look is retuned, which is
/// backwards: the conversion is a fact about a curve that no longer exists.
const RETIRED_ARC_CHROMA: f64 = 0.5;

/// The color the shipped path draws at `t` for this gradient, off the curve
/// rather than the table, so table resolution is not in the measurement.
fn sample(t: f64, gradient: Gradient) -> Vec4 {
    crate::color::designed_pitch_ramp(t, gradient)
}

/// The guarantee the whole two-space design buys: turning the CHROMA knob does
/// not move the hue. It is the promise `Gradient::hue_start` makes, and the
/// reason the ramp's hue is not simply a CIELAB angle like its lightness is an
/// `L*`.
///
/// Measured in Oklch off the drawn sRGB, which is a round trip rather than a
/// restatement: the curve names an Oklab hue, and this reads back what
/// actually reached the pixel through the gamut search, the Newton solve and
/// the transfer function. The retired CIELAB curve drifted up to 18.67 degrees
/// across this same span (see PR #224 for that measurement and issue #222 for
/// what it cost).
///
/// From 0.15 rather than 0, because hue is an angle about the neutral axis and
/// a colorless color has none — at chroma 0 the reading is whatever the last
/// bits of rounding say, which is a fact about `atan2` and not about the ramp.
#[test]
fn the_chroma_knob_does_not_move_the_hue() {
    let arcs = [
        Gradient::default(),
        Gradient { hue_start: 250.0, hue_span: 60.0, ..Gradient::default() },
        Gradient { hue_start: 20.0, hue_span: 60.0, ..Gradient::default() },
        Gradient { hue_start: 120.0, hue_span: 60.0, ..Gradient::default() },
        Gradient { lightness: 30.0, lightness_ramp: 0.0, ..Gradient::default() },
        // Steep and inverted, but stopping short of `L*` 100: a ramp that
        // clamps against the top pins several deciles at pure white, where the
        // assertion below has nothing to measure (see the guard).
        Gradient { lightness: 65.0, lightness_ramp: -60.0, ..Gradient::default() },
    ];
    for arc in arcs {
        for t in deciles() {
            let (l, h) = arc.lightness_and_hue(t);
            // Hue exists here at all. Where the gamut closes to a point — the
            // two ends of the `L*` axis — every chroma setting draws the same
            // neutral and `atan2` on (0, 0) answers 0 for each, so the
            // assertion below would compare white to white and pass without
            // asking anything. Asserted rather than skipped, so an arc added to
            // the list cannot quietly stop testing.
            assert!(
                crate::color::max_chroma_for_docs(l, h) > 1e-3,
                "{arc:?} at t {t:.1}: L* {l:.1} leaves no chroma to carry a hue, \
                 so this sample would assert nothing",
            );
            let hue_at = |c: f32| oklch(sample(t, Gradient { chroma: c, ..arc })).2;
            let anchor = hue_at(0.15);
            for chroma in [0.3, 0.5, 0.75, 1.0] {
                let drift = hue_delta(anchor, hue_at(chroma));
                // A quarter degree, which is far under the quantization the
                // color is heading for and far over the noise of the solve.
                assert!(
                    drift.abs() < 0.25,
                    "{arc:?} at t {t:.1}: chroma {chroma} moved the hue {drift:+.3} degrees",
                );
            }
        }
    }
}

/// The other half of the two-space design, and the harder half: that the join
/// between them keeps `L*`'s promise everywhere, including at the two edges
/// where the gamut closes to a point.
///
/// `L*` is a function of luminance alone, so the ramp's whole claim is that a
/// color it draws sits at exactly the luminance its `L*` names. Between the ask
/// and the pixel are a gamut bisection and a Newton solve, and this is what
/// says they deliver it.
///
/// The edges are where it is hard, and specifically the black one. The solve
/// starts from the grey of the target luminance, which at `L*` 0 is `L` 0 —
/// where the luminance curve is not flat but FALLING for a band of hues, since
/// two of its three weights are negative. An unguarded step there walks away
/// from the root and parks at a bright color whose channels are all inside the
/// sRGB box, so the gamut search grants chroma and the ramp paints a blazing
/// pixel where the curve asked for black. Only the luminance it missed says so,
/// which is why that is what this measures.
///
/// This is also the evidence for both constants the solve is built on. The gap
/// it reports between a converged solve and the tolerance is what makes
/// `LUMINANCE_STEPS` a coverage knob rather than a correctness one.
#[test]
fn the_hybrid_at_the_edges() {
    // Everything the gamut search grants, over the whole knob space: every one
    // of these is a color the shipped path will actually draw.
    let (mut worst, mut worst_at) = (0.0f64, (0.0, 0.0, 0.0));
    for l_step in 0..=100 {
        let l = f64::from(l_step);
        for h_step in 0..360 {
            let h = f64::from(h_step);
            let ceiling = crate::color::max_chroma_for_docs(l, h);
            for fraction in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let c = fraction * ceiling;
                let (target, reached) = crate::color::ramp_luminance(l, h, c);
                // Relative, for the reason `LUMINANCE_MISS` is: at the black
                // end an absolute miss is small however wrong the answer.
                // `L*` 0 is the one place the ratio has no denominator, and
                // there the only right answer is black exactly.
                let miss =
                    if target > 0.0 { (reached - target).abs() / target } else { reached.abs() };
                if miss > worst {
                    worst = miss;
                    worst_at = (l, h, c);
                }
            }
        }
    }
    // Four orders under `LUMINANCE_MISS`, which is the gap the design turns on:
    // a converged solve lands at the double's own resolution and a diverged one
    // misses by enough to see, so the tolerance between them separates the two
    // rather than trading one off against the other.
    assert!(
        worst < 1e-13,
        "the solve missed its target luminance by a relative {worst:.3e} at L* {:.0}, \
         hue {:.0}, chroma {:.4} — at that size it is no longer rounding",
        worst_at.0,
        worst_at.1,
        worst_at.2,
    );

    // And the two edges themselves. Black and white are the only colors at
    // their luminances, so the only honest answer at either end of the axis is
    // that there is no chroma to be had — the failure this guards against
    // reported up to 0.21 at `L*` 0, over a band of greens.
    for h_step in 0..3600 {
        let h = f64::from(h_step) * 0.1;
        for l in [0.0, 100.0] {
            let ceiling = crate::color::max_chroma_for_docs(l, h);
            assert!(
                ceiling == 0.0,
                "L* {l:.0} is {} and has no hue, but the gamut search grants \
                 {ceiling:.4} chroma at hue {h:.1}",
                if l == 0.0 { "black" } else { "white" },
            );
        }
    }
}

/// The Oklab hue that names the color a CIELAB hue angle used to draw — the
/// RETIRED curve, kept only so the defaults can be checked against the arc
/// they were converted from.
///
/// The chroma fraction is an argument because the answer moves with it. A
/// CIELAB hue angle is not one hue: at 20% of the gamut its color reads up to
/// 10 degrees further toward purple than the same angle at 100%, which is the
/// blue shift the ramp moved off CIELAB to escape.
fn oklab_hue_of_retired_lab_hue(l_star: f64, lab_hue: f64, chroma_fraction: f64) -> f32 {
    // The old gamut search: the largest CIELAB chroma sRGB held at this
    // lightness and hue, which is what the fraction was a fraction OF.
    let lab_in_gamut = |c: f64| {
        let rgb = color_space::Rgb::from(color_space::Lch::new(l_star, c, lab_hue));
        (0.0..=255.0).contains(&rgb.r)
            && (0.0..=255.0).contains(&rgb.g)
            && (0.0..=255.0).contains(&rgb.b)
    };
    let (mut lo, mut hi) = (0.0f64, 200.0f64);
    for _ in 0..20 {
        let mid = 0.5 * (lo + hi);
        if lab_in_gamut(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let rgb = color_space::Rgb::from(color_space::Lch::new(l_star, chroma_fraction * lo, lab_hue));
    // Through the pixel and back out with [`oklch`], which is the one
    // conversion that cannot disagree with what a CIELAB-authored ramp actually
    // puts on screen — and the same one every other measurement here reads
    // through, so the two columns of a table cannot be in different spaces by
    // accident.
    let byte = |v: f64| (v.clamp(0.0, 255.0) / 255.0) as f32;
    oklch(Vec4::new(byte(rgb.r), byte(rgb.g), byte(rgb.b), 1.0)).2 as f32
}

/// The `L*` and Oklab hue the retired CIELAB arc drew at the two ends of the
/// pitch range, for the gradient's other knobs.
fn retired_arc(lab_start: f64, lab_span: f64, g: Gradient) -> (f32, f32) {
    let end_lightness = |t: f64| {
        (f64::from(g.lightness) + (t - 0.5) * f64::from(g.lightness_ramp)).clamp(0.0, 100.0)
    };
    let start = oklab_hue_of_retired_lab_hue(end_lightness(0.0), lab_start, RETIRED_ARC_CHROMA);
    let end = oklab_hue_of_retired_lab_hue(
        end_lightness(1.0),
        (lab_start + lab_span) % 360.0,
        RETIRED_ARC_CHROMA,
    );
    // The ENDS are what convert: a hue angle's color moves with lightness and
    // chroma, so each end converts at its own end of BOTH ramps, and what lies
    // between them becomes a straight Oklab sweep — a different path through
    // the same two colors, and an evener one.
    let short = (end - start).rem_euclid(360.0);
    (start, if lab_span < 0.0 { short - 360.0 } else { short })
}

/// The defaults are the CIELAB arc the gradient used to open on, converted —
/// not a new arc someone chose. The numbers `default_hue_start` and
/// `default_hue_span` quote have no other source.
#[test]
fn the_defaults_are_the_retired_arc_converted() {
    let now = Gradient::default();
    let (start, span) = retired_arc(RETIRED_ARC_START, RETIRED_ARC_SPAN, now);
    // A tenth of a degree, which is what the defaults are rounded to. Tighter
    // would only pin the rounding; looser would stop noticing a real move.
    assert!(
        (start - now.hue_start).abs() < 0.1 && (span - now.hue_span).abs() < 0.1,
        "the defaults are no longer the retired arc: converted ({start:.4}, {span:.4}), \
         defaults ({:.4}, {:.4})",
        now.hue_start,
        now.hue_span,
    );
}

/// The chroma figures [`Gradient::chroma`]'s docs quote, measured. A doc
/// number nothing checks is a number that rots the first time the curve moves,
/// and this one is the argument for the knob being a FRACTION at all.
#[test]
fn the_default_arcs_chroma_ceiling_is_what_the_docs_say() {
    let g = Gradient::default();
    let (mut lo, mut hi) = (f64::MAX, 0.0f64);
    for k in 0..=200 {
        let (l, h) = g.lightness_and_hue(f64::from(k) / 200.0);
        let c = crate::color::max_chroma_for_docs(l, h);
        lo = lo.min(c);
        hi = hi.max(c);
    }
    assert!(
        (lo - 0.115).abs() < 0.001 && (hi - 0.320).abs() < 0.001,
        "Gradient::chroma quotes 0.115..0.320 across the default arc; it is {lo:.4}..{hi:.4}",
    );
}

/// The floor of the gamut across the whole hue circle at one `L*`, re-derived
/// rather than read out of `HUE_FLOOR` — the point being to check the table
/// against the search it was baked from.
///
/// A coarse sweep and then a refinement around the best of it, because the
/// minimum is smooth in hue but the sweep that finds it is not free: a flat
/// 0.05 degree sweep is 7200 gamut bisections per lightness, and this is asked
/// at two hundred of them.
fn measured_hue_floor(l_star: f64) -> f64 {
    let at = |h: f64| crate::color::max_chroma_for_docs(l_star, h);
    let coarse = (0..360)
        .map(f64::from)
        .min_by(|&a, &b| at(a).partial_cmp(&at(b)).expect("a bisection returns a real chroma"))
        .expect("360 hues");
    (-40..=40)
        .map(|k| at((coarse + f64::from(k) * 0.05).rem_euclid(360.0)))
        .fold(f64::INFINITY, f64::min)
}

/// `HUE_FLOOR` never rides above the gamut it stands for.
///
/// The one property the table MUST have: an entry above the true floor names a
/// chroma some hue cannot hold, and the whole design's promise is that no
/// setting of the six knobs asks for a color sRGB has to clip. Interpolation
/// between entries is what makes this worth SWEEPING rather than checking at
/// the entries alone — a chord can ride above a curve whose ends it touches.
#[test]
fn the_hue_floor_is_never_above_the_gamut() {
    let (mut worst, mut worst_at) = (f64::NEG_INFINITY, 0.0);
    let (mut slackest, mut slackest_at) = (0.0f64, 0.0);
    for k in 0..=200 {
        let l = f64::from(k) * 0.5;
        let (table, measured) = (crate::color::hue_floor_for_docs(l), measured_hue_floor(l));
        if table - measured > worst {
            worst = table - measured;
            worst_at = l;
        }
        // The other direction, which the <= above cannot see. An entry far
        // BELOW the floor is in gamut and silently pale, and the only other
        // thing pinning magnitude walks the default arc's L* 42..86 — while all
        // four heatmap presets open at L* 0 and run to 88..92, so they live
        // precisely on the entries nothing else holds down.
        if measured > 0.0 && 1.0 - table / measured > slackest {
            slackest = 1.0 - table / measured;
            slackest_at = l;
        }
    }
    assert!(
        worst <= 0.0,
        "HUE_FLOOR rides {worst:.2e} above the gamut at L*={worst_at:.1}; \
         every entry must sit at or below what every hue can hold",
    );
    // Two percent, which the 1e-4 safety drop reaches only where the floor
    // itself has closed to nearly nothing against black and white.
    assert!(
        slackest < 0.02 || crate::color::hue_floor_for_docs(slackest_at) < 0.01,
        "HUE_FLOOR gives up {:.1}% of the gamut at L*={slackest_at:.1}, where the \
         floor is {:.4}; the table has drifted low and the picture is pale for it",
        slackest * 100.0,
        crate::color::hue_floor_for_docs(slackest_at),
    );
}

/// One Chroma setting is one colorfulness, whatever hue the arc is passing
/// through — the defect this denominator exists to fix.
///
/// Stated as a bound on the SPREAD around the hue circle rather than on any one
/// hue, and checked against what a fraction of the per-hue ceiling would have
/// drawn at the same setting, so the test says what was bought rather than
/// merely that a number is small. The bound loosens with the fraction by
/// construction: `chroma_of` opens flat and reaches each hue's own ceiling at
/// 1.0, where the spread is the gamut's own and nothing can be claimed.
#[test]
fn one_chroma_setting_is_one_colorfulness_across_hue() {
    let spread = |f: &dyn Fn(f64) -> f64| {
        let cs: Vec<f64> = (0..72).map(|d| f(f64::from(d) * 5.0)).collect();
        cs.iter().cloned().fold(0.0f64, f64::max) / cs.iter().cloned().fold(f64::INFINITY, f64::min)
    };
    for l in [42.0f64, 64.0, 86.0] {
        // The last row is the fraction the type default OPENS on, which is
        // above both of the others and is where the headline claim is actually
        // made. Without it the suite would not notice the default drifting up
        // to a setting where the fix buys almost nothing.
        for (fraction, bound) in
            [(0.25f64, 1.30f64), (0.5, 1.70), (f64::from(Gradient::default().chroma), 2.15)]
        {
            let got = spread(&|h| crate::color::chroma_of_for_docs(fraction, l, h));
            let ceiling = spread(&|h| fraction * crate::color::max_chroma_for_docs(l, h));
            assert!(
                got <= bound,
                "at L*={l} and Chroma {fraction}, colorfulness spans {got:.2}x around the \
                 hue circle, over the {bound:.2}x this denominator promises",
            );
            assert!(
                got < ceiling,
                "at L*={l} and Chroma {fraction}, this denominator spans {got:.2}x where a \
                 fraction of the per-hue ceiling spans {ceiling:.2}x — the fix has stopped \
                 buying anything",
            );
        }
    }
}

/// The type default preserves the colorfulness of its denominator conversion.
/// The composed ViewConfig look is independently dialed, so its measured mean
/// is not a constraint on future appearance choices.
#[test]
fn the_default_opens_at_the_colorfulness_it_used_to() {
    let g = Gradient::default().sanitized();
    let mean = |f: &dyn Fn(f64, f64, f64) -> f64| {
        (0..=200)
            .map(|k| {
                let t = f64::from(k) / 200.0;
                let (l, h) = g.lightness_and_hue(t);
                f(l, h, g.chroma_at(t))
            })
            .sum::<f64>()
            / 201.0
    };
    let now = mean(&|l, h, fraction| crate::color::chroma_of_for_docs(fraction, l, h));
    let retired = mean(&|l, h, _| 0.5 * crate::color::max_chroma_for_docs(l, h));
    assert!(
        (now - retired).abs() / retired < 0.02,
        "the type default's mean chroma is {now:.4}, versus {retired:.4} before conversion",
    );
}
