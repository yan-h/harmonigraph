//! The two spaces the ramp is authored in: real checks that the chroma knob
//! leaves the hue alone and that the join keeps `L*`'s promise at the edges,
//! plus the probes behind #222's decision to move the ramp onto Oklab.
//!
//! Everything `#[ignore]`d here asserts nothing and prints measurements. Run
//! them with
//! `cargo test -p harmonigraph-scene hue_space -- --nocapture --ignored`.

use crate::style::PitchGradient;
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
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
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

/// CAM16's name for a drawn color's hue, read off `hct-cam16` through the same
/// byte quantization a screen applies. Only the crate's HUE is ever leaned on —
/// [`the_crate_222_names_does_not_work`] is why.
fn cam16_hue(c: Vec4) -> f64 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    hct_cam16::Hct::from_rgb(q(c.x), q(c.y), q(c.z)).hue()
}

/// The eleven deciles of `t`, which is where every table below samples.
fn deciles() -> impl Iterator<Item = f64> {
    (0..=10).map(|k| f64::from(k) / 10.0)
}

/// The CIELAB arc the shipped defaults are a conversion OF, in CIELAB degrees.
/// Named once because three things below have to agree about it: the test that
/// holds the defaults to it, and the two probes that measure what the
/// conversion does to a sweep.
const RETIRED_ARC_START: f64 = 260.0;
const RETIRED_ARC_SPAN: f64 = 190.0;

/// The color the shipped path draws at `t` for this gradient, off the curve
/// rather than the table, so table resolution is not in the measurement.
fn sample(t: f64, gradient: PitchGradient) -> Vec4 {
    crate::color::designed_pitch_ramp(t, gradient)
}

/// One color of the shipped curve by its coordinates: an `L*`, an Oklab hue,
/// and a chroma fraction. A gradient of no span, read at its middle.
fn at(l_star: f64, hue: f64, chroma_fraction: f64) -> Vec4 {
    sample(
        0.5,
        PitchGradient {
            hue_start: hue as f32,
            hue_span: 0.0,
            lightness: l_star as f32,
            lightness_ramp: 0.0,
            chroma: chroma_fraction as f32,
        },
    )
}

/// The guarantee the whole two-space design buys: turning the CHROMA knob does
/// not move the hue. A real check and not a probe — it is the promise
/// `PitchGradient::hue_start` makes, and the reason the ramp's hue is not
/// simply a CIELAB angle like its lightness is an `L*`.
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
        PitchGradient::default(),
        PitchGradient { hue_start: 250.0, hue_span: 60.0, ..PitchGradient::default() },
        PitchGradient { hue_start: 20.0, hue_span: 60.0, ..PitchGradient::default() },
        PitchGradient { hue_start: 120.0, hue_span: 60.0, ..PitchGradient::default() },
        PitchGradient { lightness: 30.0, lightness_ramp: 0.0, ..PitchGradient::default() },
        // Steep and inverted, but stopping short of `L*` 100: a ramp that
        // clamps against the top pins several deciles at pure white, where the
        // assertion below has nothing to measure (see the guard).
        PitchGradient { lightness: 65.0, lightness_ramp: -60.0, ..PitchGradient::default() },
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
            let hue_at = |c: f32| oklch(sample(t, PitchGradient { chroma: c, ..arc })).2;
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

/// The evenness the move to Oklab buys: equal steps of hue ANGLE should be
/// equal steps of the picture. Measured as how far each decile has come along
/// the arc, against the share of the arc it names.
///
/// The angles stepped through here are CIELAB's, over the arc the defaults were
/// converted from (260, span 190) — which is the only way the question has an
/// answer. Stepping Oklab angles and reading Oklab hues back measures a space
/// against itself and reports 1.0x by construction, whatever either space is
/// like; what can be compared is one space's even steps seen through the other.
#[test]
#[ignore = "a probe: prints measurements, asserts nothing"]
fn equal_lab_steps_are_uneven_in_oklch() {
    for chroma in [0.2, 0.5, 1.0] {
        let arc = PitchGradient { chroma, ..PitchGradient::default() };
        println!("\n=== default arc at chroma {:.0}%: evenness of the sweep ===", chroma * 100.0);
        println!("   t  Lab h   ok h     step   share of arc   (even would be)");
        // The retired arc's own angles, at the lightness this ramp puts each
        // decile at — a CIELAB hue's color moves with lightness, so the two
        // have to be read together.
        let lab_hue = |t: f64| RETIRED_ARC_START + t * RETIRED_ARC_SPAN;
        let hues: Vec<f64> = deciles()
            .map(|t| {
                let l = arc.lightness_and_hue(t).0;
                f64::from(oklab_hue_of_retired_lab_hue(l, lab_hue(t), f64::from(chroma)))
            })
            .collect();
        // Unwrapped as it goes, so an arc crossing 0 stays monotone and the
        // shares below are cumulative distance rather than an angle.
        let mut walked = vec![0.0];
        for k in 1..hues.len() {
            let step = hue_delta(hues[k - 1], hues[k]);
            walked.push(walked[k - 1] + step);
        }
        let total = *walked.last().expect("eleven samples");
        let (mut min_step, mut max_step) = (f64::MAX, f64::MIN);
        for (k, t) in deciles().enumerate() {
            let lab_h = lab_hue(t);
            let step = if k == 0 { 0.0 } else { walked[k] - walked[k - 1] };
            if k > 0 {
                min_step = min_step.min(step);
                max_step = max_step.max(step);
            }
            println!(
                "{t:4.1}  {lab_h:6.1}  {:6.1}  {step:7.2}  {:11.3}   {t:14.3}",
                hues[k],
                walked[k] / total,
            );
        }
        println!(
            "steps run {min_step:.2}..{max_step:.2} degrees of Oklch hue per equal Lab step \
             ({:.1}x)",
            max_step / min_step,
        );
    }
}

/// What the CIELAB axis makes of the circle the ramp now walks directly, so the
/// compression and stretch it introduces have somewhere to be read off. The
/// retired arc's own region (CIELAB 260 round to 90) is what #222 was about.
///
/// A stretch of 1.0 everywhere would mean the two axes name the same circle and
/// the move bought nothing; the reason this is worth printing is that they do
/// not. Both columns must therefore come from different spaces — feeding an
/// Oklab hue in and reading an Oklab hue back is the identity function, and
/// would print 1.000 down the page whatever the spaces were like.
#[test]
#[ignore = "a probe: prints measurements, asserts nothing"]
fn the_hue_circle_in_both_spaces() {
    println!("\n=== CIELAB hue -> Oklch hue at L* 64, chroma 50% ===");
    println!("  Lab h   ok h    local stretch (deg ok per deg Lab)");
    let hue_of = |h: f64| f64::from(oklab_hue_of_retired_lab_hue(64.0, h, 0.5));
    for step in 0..36 {
        let h = f64::from(step) * 10.0;
        let stretch = hue_delta(hue_of(h - 5.0), hue_of(h + 5.0)) / 10.0;
        let on_arc = h >= RETIRED_ARC_START || h <= RETIRED_ARC_START + RETIRED_ARC_SPAN - 360.0;
        let marker = if on_arc { " <- retired arc" } else { "" };
        println!("{h:7.1}  {:6.1}  {stretch:8.3}{marker}", hue_of(h));
    }
}

/// Whether the Oklab hybrid above and Google's HCT are the same picture.
///
/// They are the same STRUCTURE — tone from CIELAB `L*`, hue and chroma from a
/// space with good hue constancy — and differ only in which space that is:
/// CAM16 for HCT (`cam16_hue_chroma_from_argb` + `lstar_from_argb`, read off
/// the crate), Oklab here. So the question is not which is better designed but
/// whether their hue axes agree, since one costs a CAM16 conversion per
/// bisection step and the other does not.
///
/// Measured through CAM16's own eyes, which is the harshest way round: if a
/// hybrid that never mentions CAM16 holds a CAM16 hue still, the two agree.
/// Only the crate's HUE is leaned on here, and
/// [`the_crate_222_names_does_not_work`] is why — its chroma is broken, but
/// hue falls out of `atan2` on the opponent signals before the chroma formula
/// runs, so the two failures are not the same failure.
///
/// Neither column is ground truth. A space measured in its own coordinates
/// always reads perfect (the hybrid drifts 0.0001 degrees in Oklch, by
/// construction), so what this can settle is whether the two DISAGREE, not
/// which is right.
#[test]
#[ignore = "a probe: prints measurements, asserts nothing"]
fn the_oklab_hybrid_against_googles_hct() {
    let arc = PitchGradient::default();

    println!("\n=== CAM16 hue drift over the shipped chroma knob (20% -> 100%) ===");
    println!("   t   L*    Oklab hue held    CAM16 drift");
    let mut worst = 0.0f64;
    for t in deciles() {
        let (l, h) = arc.lightness_and_hue(t);
        let drift = hue_delta(cam16_hue(at(l, h, 0.2)), cam16_hue(at(l, h, 1.0)));
        worst = worst.max(drift.abs());
        println!("{t:4.1} {l:5.1} {h:16.1} {drift:+14.2}");
    }
    println!(
        "worst: {worst:.2} deg — the retired CIELAB curve measured 13.65 here, \n\
         and 0.0001 in Oklch's own coordinates, which is why neither is ground truth",
    );

    println!("\n=== where the two hue axes sit, around the circle at L* 64 ===");
    println!("  ok h    CAM16 h    difference");
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for step in 0..24 {
        let h_ok = f64::from(step) * 15.0;
        let gap = hue_delta(h_ok, cam16_hue(at(64.0, h_ok, 0.6)));
        lo = lo.min(gap);
        hi = hi.max(gap);
        println!("{h_ok:6.1} {:10.1} {gap:13.1}", h_ok + gap);
    }
    // The VARIATION and not the gap itself: a constant rotation between two hue
    // axes renames every hue and bends no arc, so only the spread can change a
    // picture.
    println!("gap runs {lo:.1}..{hi:.1} deg — a spread of {:.1}", hi - lo);
}

/// Why the crate #222 names as "the exact piece `max_chroma` hand-rolls"
/// cannot play that part: `hct-cam16` 0.1.0's chroma is wrong, and its
/// HCT->sRGB solver does not round-trip.
///
/// Three checks that need no reference values to read. Tone is fine and hue
/// looks right; it is chroma and the solver that fail, which matters here
/// because chroma and the solver are precisely the piece the issue proposes to
/// take from it.
#[test]
#[ignore = "a probe: prints measurements, asserts nothing"]
fn the_crate_222_names_does_not_work() {
    println!("\n=== hct-cam16 0.1.0: the most colorful colors sRGB has ===");
    println!("(the crate documents chroma as [0, ~150]; CAM16 puts sRGB red near 110)");
    for (name, hex) in [
        ("red", "#FF0000"), ("green", "#00FF00"), ("blue", "#0000FF"),
        ("magenta", "#FF00FF"), ("cyan", "#00FFFF"), ("yellow", "#FFFF00"),
        ("mid grey", "#808080"), ("M3 seed", "#6750A4"),
    ] {
        let c = hct_cam16::Hct::from_hex(hex).expect("literal hex");
        println!("  {name:9} {hex}  h {:6.2}  c {:6.2}  t {:6.2}", c.hue(), c.chroma(), c.tone());
    }

    println!("\n=== and asking for a color's own coordinates does not return it ===");
    for hex in ["#FF0000", "#00FF00", "#6750A4"] {
        let a = hct_cam16::Hct::from_hex(hex).expect("literal hex");
        let b = hct_cam16::Hct::new(a.hue(), a.chroma(), a.tone());
        println!(
            "  {hex} reads h{:.1} c{:.1} t{:.1}, and that asked for again is {} (c{:.1})",
            a.hue(), a.chroma(), a.tone(), b.to_hex(), b.chroma(),
        );
    }
    println!(
        "\nTone is right and hue is plausible: hue is atan2 on the opponent signals,\n\
         computed before the chroma formula, so the two do not fail together.",
    );
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
/// pitch range, for the gradient's other three knobs.
fn retired_arc(lab_start: f64, lab_span: f64, g: PitchGradient) -> (f32, f32) {
    let end_lightness = |t: f64| {
        (f64::from(g.lightness) + (t - 0.5) * f64::from(g.lightness_ramp)).clamp(0.0, 100.0)
    };
    let chroma = f64::from(g.chroma);
    let start = oklab_hue_of_retired_lab_hue(end_lightness(0.0), lab_start, chroma);
    let end =
        oklab_hue_of_retired_lab_hue(end_lightness(1.0), (lab_start + lab_span) % 360.0, chroma);
    // The ENDS are what convert: a hue angle's color moves with lightness and
    // chroma, so each end converts at its own end of the ramp, and what lies
    // between them becomes a straight Oklab sweep — a different path through
    // the same two colors, and an evener one.
    let short = (end - start).rem_euclid(360.0);
    (start, if lab_span < 0.0 { short - 360.0 } else { short })
}

/// The defaults are the CIELAB arc the gradient used to open on, converted —
/// not a new arc someone chose. Real rather than a probe, because the numbers
/// `default_hue_start` and `default_hue_span` quote have no other source.
#[test]
fn the_defaults_are_the_retired_arc_converted() {
    let now = PitchGradient::default();
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

/// The chroma figures [`PitchGradient::chroma`]'s docs quote, measured. A doc
/// number nothing checks is a number that rots the first time the curve moves,
/// and this one is the argument for the knob being a FRACTION at all.
#[test]
fn the_default_arcs_chroma_ceiling_is_what_the_docs_say() {
    let g = PitchGradient::default();
    let (mut lo, mut hi) = (f64::MAX, 0.0f64);
    for k in 0..=200 {
        let (l, h) = g.lightness_and_hue(f64::from(k) / 200.0);
        let c = crate::color::max_chroma_for_docs(l, h);
        lo = lo.min(c);
        hi = hi.max(c);
    }
    assert!(
        (lo - 0.115).abs() < 0.001 && (hi - 0.320).abs() < 0.001,
        "PitchGradient::chroma quotes 0.115..0.320 across the default arc; it is {lo:.4}..{hi:.4}",
    );
}

/// What moving on again — from Oklab to CAM16, i.e. to Google's HCT proper —
/// would actually change about the default arc.
///
/// Not the hue NAMES, which differ by up to 5.3 degrees around the circle and
/// cost nothing: a constant rotation between two hue axes renames every hue
/// and bends no arc. What a viewer could see is the SHAPE — whether the sweep
/// that is even in Oklab arrives somewhere else at the same fraction of the
/// pitch range under CAM16.
///
/// Measured as: how far along the arc each decile has come by each space's
/// reckoning, differenced. Leans only on the crate's hue, for the reason
/// [`the_crate_222_names_does_not_work`] gives.
#[test]
#[ignore = "a probe: prints measurements, asserts nothing"]
fn what_going_on_to_cam16_would_buy() {
    let arc = PitchGradient::default();
    // Unwrapped as it goes, so an arc crossing 0 stays monotone and the shares
    // below are cumulative distance rather than an angle.
    let samples: Vec<f64> = (0..=100)
        .map(|k| {
            let (l, h) = arc.lightness_and_hue(f64::from(k) / 100.0);
            cam16_hue(at(l, h, f64::from(arc.chroma)))
        })
        .collect();
    let mut walked = vec![0.0f64];
    for k in 1..samples.len() {
        let step = hue_delta(samples[k - 1], samples[k]);
        walked.push(walked[k - 1] + step);
    }
    let total = *walked.last().expect("101 samples");

    println!("\n=== the default arc, walked by each space's own hue ===");
    println!("   t   share of arc: Oklab   CAM16    difference (of the pitch range)");
    let mut worst = 0.0f64;
    for k in (0..=100).step_by(10) {
        // Oklab's own reckoning is `t` exactly: the sweep is linear in the hue
        // the curve names, which is what makes it even.
        let t = k as f64 / 100.0;
        let share = walked[k] / total;
        let diff = share - t;
        worst = worst.max(diff.abs());
        println!("{t:4.1} {t:23.3} {share:8.3} {:13.1}%", diff * 100.0);
    }
    println!(
        "worst disagreement about where a pitch sits in the sweep: {:.1}% of the range\n\
         (the retired CIELAB curve was off by 4.8% at this chroma and 5.8% at full,\n\
         so what is left is about a third of what the move to Oklab already fixed —\n\
         and unlike that, it is two good spaces disagreeing rather than a defect)",
        worst * 100.0,
    );
}
