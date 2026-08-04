//! The probe #222 asks for: how far the pitch gradient's hue drifts, in a
//! space with good hue constancy, when only its chroma knob moves.
//!
//! Not an assertion about the design — a measurement, printed. Run it with
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

/// The eleven deciles of `t`, which is where every table below samples.
fn deciles() -> impl Iterator<Item = f64> {
    (0..=10).map(|k| f64::from(k) / 10.0)
}

/// The color the shipped path draws at `t` for this gradient, off the curve
/// rather than the table, so table resolution is not in the measurement.
fn sample(t: f64, gradient: PitchGradient) -> Vec4 {
    crate::color::designed_pitch_ramp(t, gradient)
}

/// #222's decisive experiment: the same arc at two chroma settings, compared
/// in Oklch. Under a space with good hue constancy the hue angle barely moves;
/// under CIELAB it drifts, worst in the blues.
#[test]
#[ignore = "a probe: prints measurements, asserts nothing"]
fn chroma_drifts_the_hue_it_should_not_touch() {
    let arc =
        |hue_start, hue_span| PitchGradient { hue_start, hue_span, ..PitchGradient::default() };
    let arcs = [
        ("default (260 + 190)", PitchGradient::default()),
        ("blues only (250 + 60)", arc(250.0, 60.0)),
        ("warm control (20 + 60)", arc(20.0, 60.0)),
        ("green control (120 + 60)", arc(120.0, 60.0)),
    ];
    for (name, arc) in arcs {
        println!("\n=== {name}: Oklch hue as the chroma knob moves ===");
        println!("   t   Lab h    ok h @20%   @50%    @100%   drift 20-50  drift 20-100");
        let (mut worst_half, mut worst_full) = (0.0f64, 0.0f64);
        for t in deciles() {
            let (_, lab_h) = arc.lightness_and_hue(t);
            let at = |c: f32| oklch(sample(t, PitchGradient { chroma: c, ..arc })).2;
            let (h_lo, h_mid, h_hi) = (at(0.2), at(0.5), at(1.0));
            let (d_half, d_full) = (hue_delta(h_lo, h_mid), hue_delta(h_lo, h_hi));
            worst_half = worst_half.max(d_half.abs());
            worst_full = worst_full.max(d_full.abs());
            println!(
                "{t:4.1}  {lab_h:6.1}  {h_lo:10.2} {h_mid:7.2} {h_hi:8.2}  \
                 {d_half:+11.2}  {d_full:+12.2}",
            );
        }
        println!("worst drift: {worst_half:.2} deg over 20-50%, {worst_full:.2} deg over 20-100%");
    }
}

/// The half of the blue shift the Brightness knob reaches, which #222 does not
/// ask about and which is dragged more often than Chroma: a constant CIELAB
/// hue does not hold its perceived hue as `L*` moves either.
#[test]
#[ignore = "a probe: prints measurements, asserts nothing"]
fn brightness_drifts_the_hue_as_well() {
    println!("\n=== Oklch hue of one CIELAB hue as L* runs 20..90, chroma 50% ===");
    println!("  Lab h    L*20    L*35    L*50    L*65    L*80    L*90    spread");
    for lab_h in [200.0, 230.0, 260.0, 280.0, 300.0, 330.0, 0.0, 30.0, 60.0, 90.0, 140.0] {
        let at = |l: f32| {
            let g = PitchGradient {
                hue_start: lab_h,
                hue_span: 0.0,
                lightness: l,
                lightness_ramp: 0.0,
                chroma: 0.5,
            };
            oklch(sample(0.5, g)).2
        };
        let hs: Vec<f64> = [20.0, 35.0, 50.0, 65.0, 80.0, 90.0].iter().map(|&l| at(l)).collect();
        // Against the mid-lightness sample rather than the widest pair, so the
        // figure is how far the hue wanders from where the knob's middle puts it.
        let spread = hs.iter().map(|&h| hue_delta(hs[2], h).abs()).fold(0.0f64, f64::max);
        println!(
            "{lab_h:7.1} {:7.1} {:7.1} {:7.1} {:7.1} {:7.1} {:7.1}  {spread:8.2}",
            hs[0], hs[1], hs[2], hs[3], hs[4], hs[5],
        );
    }
}

/// The arc's other claim: equal steps of CIELAB hue ANGLE are meant to be
/// equal steps of the picture. Measured as how far each decile has come along
/// the arc in Oklch, against the share of the arc it names.
#[test]
#[ignore = "a probe: prints measurements, asserts nothing"]
fn equal_lab_steps_are_uneven_in_oklch() {
    for chroma in [0.2, 0.5, 1.0] {
        let arc = PitchGradient { chroma, ..PitchGradient::default() };
        println!("\n=== default arc at chroma {:.0}%: evenness of the sweep ===", chroma * 100.0);
        println!("   t   Lab h   ok h     step   share of arc   (even would be)");
        let hues: Vec<f64> = deciles().map(|t| oklch(sample(t, arc)).2).collect();
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
            let (_, lab_h) = arc.lightness_and_hue(t);
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

/// What the two hue axes make of the same arc, side by side: the whole circle
/// of CIELAB hue in Oklch, so the compression and stretch have somewhere to be
/// read off. The default arc's own region is what #222 is about.
#[test]
#[ignore = "a probe: prints measurements, asserts nothing"]
fn the_hue_circle_in_both_spaces() {
    let base = PitchGradient { lightness: 64.0, lightness_ramp: 0.0, ..PitchGradient::default() };
    println!("\n=== CIELAB hue -> Oklch hue at L* 64, chroma 50% ===");
    println!("  Lab h   ok h    local stretch (deg ok per deg Lab)");
    let at = |h: f64| {
        let g = PitchGradient { hue_start: h as f32, hue_span: 0.0, chroma: 0.5, ..base };
        oklch(sample(0.5, g)).2
    };
    for step in 0..36 {
        let h = f64::from(step) * 10.0;
        let stretch = hue_delta(at(h - 5.0), at(h + 5.0)) / 10.0;
        let marker = if (260.0..=360.0).contains(&h) || h <= 90.0 { " <- default arc" } else { "" };
        println!("{h:7.1}  {:6.1}  {stretch:8.3}{marker}", at(h));
    }
}

// ---------------------------------------------------------------------------
// A scratch build of the hybrid #222 raises as "HCT": tone from CIELAB `L*`,
// hue and chroma from a space with good hue constancy — Oklab here, where the
// issue reaches for CAM16. None of this is shipped code; it exists to put a
// cost and a pair of guarantees on the option before anyone commits to it.
// ---------------------------------------------------------------------------

/// The luminance one `L*` names. `L*` is a function of Y alone, which is the
/// whole reason this hybrid can exist.
fn y_of_l_star(l_star: f64) -> f64 {
    let fy = (l_star + 16.0) / 116.0;
    if fy > 6.0 / 29.0 { fy.powi(3) } else { 3.0 * (6.0 / 29.0f64).powi(2) * (fy - 4.0 / 29.0) }
}

/// Linear sRGB from Oklab, and the relative luminance that comes with it.
fn linear_srgb_of_oklab(ok_l: f64, ok_a: f64, ok_b: f64) -> (f64, f64, f64) {
    let l = (ok_l + 0.3963377774 * ok_a + 0.2158037573 * ok_b).powi(3);
    let m = (ok_l - 0.1055613458 * ok_a - 0.0638541728 * ok_b).powi(3);
    let s = (ok_l - 0.0894841775 * ok_a - 1.2914855480 * ok_b).powi(3);
    (
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    )
}

/// The Oklab `L` that puts a given Oklab hue and chroma at a given luminance.
///
/// Newton rather than a second bisection, which is what keeps this affordable:
/// at a fixed hue the three cube roots are linear in `L`, so Y is a CUBIC in
/// `L` with a derivative in closed form and five steps land on it. A nested
/// bisection would cost twenty.
fn ok_l_for_luminance(h_ok: f64, c_ok: f64, target_y: f64) -> f64 {
    let (cos_h, sin_h) = (h_ok.to_radians().cos(), h_ok.to_radians().sin());
    let k = [
        0.3963377774 * cos_h + 0.2158037573 * sin_h,
        -0.1055613458 * cos_h - 0.0638541728 * sin_h,
        -0.0894841775 * cos_h - 1.2914855480 * sin_h,
    ];
    // Y as a weighted sum of the three cubed roots, the weights being sRGB's
    // luminance row folded through the LMS->linear-RGB matrix.
    let w = [
        0.2126 * 4.0767416621 + 0.7152 * -1.2684380046 + 0.0722 * -0.0041960863,
        0.2126 * -3.3077115913 + 0.7152 * 2.6097574011 + 0.0722 * -0.7034186147,
        0.2126 * 0.2309699292 + 0.7152 * -0.3413193965 + 0.0722 * 1.7076147010,
    ];
    let mut ok_l = target_y.cbrt().clamp(0.0, 1.0);
    for _ in 0..5 {
        let r = [ok_l + k[0] * c_ok, ok_l + k[1] * c_ok, ok_l + k[2] * c_ok];
        let y: f64 = (0..3).map(|i| w[i] * r[i].powi(3)).sum();
        let dy: f64 = (0..3).map(|i| 3.0 * w[i] * r[i].powi(2)).sum();
        if dy.abs() < 1e-12 {
            break;
        }
        ok_l = (ok_l - (y - target_y) / dy).clamp(0.0, 1.5);
    }
    ok_l
}

/// The hybrid's own gamut ceiling: the largest Oklab chroma that stays inside
/// sRGB once `L` has been pulled back to the luminance `L*` asks for. Same
/// twenty-step bisection as [`max_chroma`], around a different inner solve.
fn hybrid_max_chroma(l_star: f64, h_ok: f64) -> f64 {
    let target_y = y_of_l_star(l_star);
    let (mut lo, mut hi) = (0.0f64, 0.4f64);
    for _ in 0..20 {
        let mid = 0.5 * (lo + hi);
        let ok_l = ok_l_for_luminance(h_ok, mid, target_y);
        let (cos_h, sin_h) = (h_ok.to_radians().cos(), h_ok.to_radians().sin());
        let (r, g, b) = linear_srgb_of_oklab(ok_l, mid * cos_h, mid * sin_h);
        let ok = |v: f64| (-1e-9..=1.0 + 1e-9).contains(&v);
        if ok(r) && ok(g) && ok(b) { lo = mid } else { hi = mid }
    }
    lo
}

/// One hybrid color, as the shipped curve would build it: `L*` for tone, an
/// Oklab hue, and a chroma that is a fraction of what the gamut holds there.
fn hybrid_color(l_star: f64, h_ok: f64, chroma_frac: f64) -> Vec4 {
    let c_ok = chroma_frac * hybrid_max_chroma(l_star, h_ok);
    let ok_l = ok_l_for_luminance(h_ok, c_ok, y_of_l_star(l_star));
    let (r, g, b) =
        linear_srgb_of_oklab(ok_l, c_ok * h_ok.to_radians().cos(), c_ok * h_ok.to_radians().sin());
    let enc = |v: f64| {
        let v = v.clamp(0.0, 1.0);
        (if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }) as f32
    };
    Vec4::new(enc(r), enc(g), enc(b), 1.0)
}

/// What the hybrid buys and what it costs, against the shipped CIELAB path:
/// the same two guarantees measured the same way, and a rebuild timed.
#[test]
#[ignore = "a probe: prints measurements, asserts nothing"]
fn the_hybrid_keeps_both_promises_and_what_it_costs() {
    // The default arc's hues, read across to Oklab so the two arcs are the
    // same picture rather than the same numbers.
    let arc = PitchGradient::default();
    println!("\n=== hybrid (L* tone, Oklab hue): does the chroma knob still drift hue? ===");
    println!("   t    ok h @20%   @50%   @100%   drift 20-100");
    let mut worst: f64 = 0.0;
    for t in deciles() {
        let (l, _lab_h) = arc.lightness_and_hue(t);
        // The Oklab hue this Lab hue names at the arc's own mid chroma, so the
        // hybrid arc opens on the same colors the current one does.
        let h_ok = oklch(sample(t, arc)).2;
        let at = |f: f64| oklch(hybrid_color(l, h_ok, f)).2;
        let (h_lo, h_mid, h_hi) = (at(0.2), at(0.5), at(1.0));
        let d = hue_delta(h_lo, h_hi);
        worst = worst.max(d.abs());
        println!("{t:4.1} {h_lo:10.2} {h_mid:7.2} {h_hi:8.2}  {d:+12.4}");
    }
    println!("worst drift: {worst:.4} degrees (CIELAB's is 18.67)");

    // Isoluminance, the guarantee #220 built the knobs on, measured the way
    // `the_gradient_is_in_gamut_and_flat_when_its_ramp_is` measures it.
    let lum = |c: Vec4| {
        let lin = |v: f32| {
            let v = f64::from(v);
            if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(c.x) + 0.7152 * lin(c.y) + 0.0722 * lin(c.z)
    };
    println!("\n=== hybrid: luminance across a flat ramp (L* 64), chroma 100% ===");
    let flat = PitchGradient { lightness_ramp: 0.0, chroma: 1.0, ..PitchGradient::default() };
    let (mut lo, mut hi) = (f64::MAX, 0.0f64);
    for t in deciles() {
        let (l, _) = flat.lightness_and_hue(t);
        let h_ok = oklch(sample(t, flat)).2;
        let y = lum(hybrid_color(l, h_ok, 1.0));
        lo = lo.min(y);
        hi = hi.max(y);
    }
    println!("luminance ratio hi/lo across the arc: {:.6} (CIELAB's is 1.000)", hi / lo);

    // Cost. A rebuild is PITCH_LUT_N entries plus, while a brightness or
    // chroma knob is dragged, the HUE_CIRCLE_N the pane draws beside it.
    let entries = crate::PITCH_LUT_N + crate::color::HUE_CIRCLE_N;
    let reps = 200;
    let t0 = std::time::Instant::now();
    let mut sink = 0.0f32;
    for r in 0..reps {
        let nudge = f64::from(r % 7) * 0.01;
        for k in 0..entries {
            let t = k as f64 / (entries - 1) as f64;
            sink += hybrid_color(40.0 + 40.0 * t + nudge, 360.0 * t, 0.5).x;
        }
    }
    let hybrid_ns = t0.elapsed().as_secs_f64() / f64::from(reps) * 1e6;
    let t1 = std::time::Instant::now();
    for r in 0..reps {
        let nudge = f64::from(r % 7) * 0.01;
        for k in 0..entries {
            let t = k as f64 / (entries - 1) as f64;
            let g = PitchGradient {
                hue_start: (360.0 * t) as f32,
                hue_span: 0.0,
                lightness: (40.0 + 40.0 * t + nudge) as f32,
                lightness_ramp: 0.0,
                chroma: 0.5,
            };
            sink += sample(0.5, g).x;
        }
    }
    let cielab_ns = t1.elapsed().as_secs_f64() / f64::from(reps) * 1e6;
    println!(
        "\n=== cost per drag frame ({entries} samples: LUT + hue circle) ===\n\
         CIELAB (shipped): {cielab_ns:8.1} us\n\
         hybrid:           {hybrid_ns:8.1} us  ({:.2}x)\n\
         (sink {sink})",
        hybrid_ns / cielab_ns,
    );
}

/// The hybrid measured where an iterative solve is most likely to come apart:
/// the ends of the `L*` axis, a zero chroma, and every hue — the sweep a real
/// implementation would have to survive. Gamut and luminance both, since the
/// two are one guarantee (see the shipped test of the same name).
#[test]
#[ignore = "a probe: prints measurements, asserts nothing"]
fn the_hybrid_at_the_edges() {
    let lum = |c: Vec4| {
        let lin = |v: f32| {
            let v = f64::from(v);
            if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(c.x) + 0.7152 * lin(c.y) + 0.0722 * lin(c.z)
    };
    let (mut worst_y_err, mut worst_at) = (0.0f64, (0.0, 0.0, 0.0));
    let mut clipped = 0;
    for l_step in 0..=100 {
        let l_star = f64::from(l_step);
        let want_y = y_of_l_star(l_star);
        for h_step in 0..72 {
            let h = f64::from(h_step) * 5.0;
            for frac in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let c = hybrid_color(l_star, h, frac);
                // Relative, since an absolute luminance error means nothing
                // next to a target Y that runs over four orders of magnitude.
                let err = if want_y > 1e-9 { (lum(c) - want_y).abs() / want_y } else { lum(c) };
                if err > worst_y_err {
                    worst_y_err = err;
                    worst_at = (l_star, h, frac);
                }
                // A channel sitting exactly on an end is CORRECT at chroma
                // 100% (that is the gamut boundary the bisection is looking
                // for) and at the ends of the `L*` axis. Anywhere else it
                // would mean the solve overshot.
                let pinned = [c.x, c.y, c.z].iter().any(|v| *v <= 0.0 || *v >= 1.0);
                if pinned && frac < 1.0 && l_star > 0.0 && l_star < 100.0 {
                    clipped += 1;
                }
            }
        }
    }
    let (l, h, f) = worst_at;
    println!(
        "\n=== hybrid over the whole knob space (101 L* x 72 hues x 5 chromas) ===\n\
         worst relative luminance error: {:.3e} at L* {l}, hue {h}, chroma {f}\n\
         samples pinned to a channel end: {clipped}",
        worst_y_err,
    );

    // The real rebuild, both ways: PITCH_LUT_N entries with the memo defeated
    // by a gradient that differs every rep.
    let reps = 300;
    let t0 = std::time::Instant::now();
    let mut sink = 0.0f32;
    for r in 0..reps {
        let lightness = 50.0 + (r % 17) as f32 * 0.1;
        let g = PitchGradient { lightness, ..PitchGradient::default() };
        sink += crate::color::pitch_ramp_lut(g)[7].x;
    }
    let cielab_us = t0.elapsed().as_secs_f64() / f64::from(reps) * 1e6;
    let t1 = std::time::Instant::now();
    for r in 0..reps {
        let base = 50.0 + f64::from(r % 17) * 0.1;
        for k in 0..crate::PITCH_LUT_N {
            let t = k as f64 / (crate::PITCH_LUT_N - 1) as f64;
            sink += hybrid_color(base + (t - 0.5) * 44.0, (246.9 + t * 200.0) % 360.0, 0.5).x;
        }
    }
    let hybrid_us = t1.elapsed().as_secs_f64() / f64::from(reps) * 1e6;
    println!(
        "\n=== one {}-entry LUT rebuild ===\n\
         CIELAB (shipped pitch_ramp_lut): {cielab_us:7.1} us\n\
         hybrid (L* tone, Oklab hue):     {hybrid_us:7.1} us  ({:.2}x)\n\
         (sink {sink})",
        crate::PITCH_LUT_N,
        hybrid_us / cielab_us,
    );
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
    let cam16_hue = |c: Vec4| {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        hct_cam16::Hct::from_rgb(q(c.x), q(c.y), q(c.z)).hue()
    };
    let arc = PitchGradient::default();

    println!("\n=== CAM16 hue drift over the chroma knob (20% -> 100%) ===");
    println!("   t   L*    CIELAB (shipped)   Oklab hybrid");
    let (mut w_lab, mut w_ok) = (0.0f64, 0.0f64);
    for t in deciles() {
        let (l, _) = arc.lightness_and_hue(t);
        // The shipped path, whose hue is a CIELAB angle held fixed.
        let lab = |f: f32| cam16_hue(sample(t, PitchGradient { chroma: f, ..arc }));
        // The hybrid, whose hue is an Oklab angle held fixed.
        let h_ok = oklch(sample(t, arc)).2;
        let hyb = |f: f64| cam16_hue(hybrid_color(l, h_ok, f));
        let d_lab = hue_delta(lab(0.2), lab(1.0));
        let d_ok = hue_delta(hyb(0.2), hyb(1.0));
        w_lab = w_lab.max(d_lab.abs());
        w_ok = w_ok.max(d_ok.abs());
        println!("{t:4.1} {l:5.1} {d_lab:+16.2} {d_ok:+15.2}");
    }
    println!("worst: CIELAB {w_lab:.2} deg, Oklab hybrid {w_ok:.2} deg");

    println!("\n=== where the two hue axes sit, around the circle at L* 64 ===");
    println!("  ok h    CAM16 h    difference");
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for step in 0..24 {
        let h_ok = f64::from(step) * 15.0;
        let gap = hue_delta(h_ok, cam16_hue(hybrid_color(64.0, h_ok, 0.6)));
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
