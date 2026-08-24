//! What a sweep peak is worth on top of the colour a layer already has.

use crate::*;
use super::fixtures::*;

/// Where a pixel sits on the hue circle in degrees, or `None` for one too near
/// grey to have a hue at all.
///
/// The hexcone hue rather than a perceptual one, for the same reason the chroma
/// reading below is a channel spread: what it has to do is move when the color
/// changes hue and hold still when the color only gets lighter or paler, and
/// both shots are read the same way. The grey guard is what keeps it honest —
/// hue is undefined on the achromatic axis, so a pixel washed out to white
/// would otherwise report an arbitrary angle and be counted as a rotation.
fn hue_degrees(px: &[u8]) -> Option<f64> {
    let (r, g, b) = (f64::from(px[0]), f64::from(px[1]), f64::from(px[2]));
    let (high, low) = (r.max(g).max(b), r.min(g).min(b));
    let c = high - low;
    if c < 8.0 {
        return None;
    }
    let h = if high == r {
        (g - b) / c
    } else if high == g {
        (b - r) / c + 2.0
    } else {
        (r - g) / c + 4.0
    };
    Some((h * 60.0).rem_euclid(360.0))
}

/// One node wearing both rings in `color`, shot steady and then at eight
/// moments of one period of the sweep — the same geometry every time, so two
/// colors' readings line up pixel for pixel and a difference between them is a
/// difference the COLOR made.
///
/// Eight moments rather than one because the sheet is a plane crossing the
/// lattice: which part of the ring a crest is over depends on where the node
/// sits under it, and no single instant has every pixel at its own peak. Both
/// halves of the period come off the scene the fixture actually builds, so a
/// caller that retunes either bar still gets one whole cycle rather than eight
/// arbitrary phases of a longer one.
///
/// Eight samples leave the sampled peak a little under the true one: the worst
/// phase offset is an eighth of a turn, which puts `wave` at 0.962, and the
/// band the shader draws is `pow(wave, sharpness)` — 0.943 at this fixture's
/// Softness. Both colors are sampled at the same phases, so that 5.7% cancels
/// between them and none of it reaches a comparison.
fn sweep_over_color(gpu: &mut Shooter, color: glam::Vec4) -> Shots {
    let at = |pulse, time: f64| -> Scene {
        let mut scene = single_marked_node(MIDDLE_C, MIDDLE_C);
        scene.nodes[0].melody_color = color;
        scene.nodes[0].bass_color = color;
        scene.pulse_marks = pulse;
        scene.now = time;
        scene
    };
    let steady_scene = at(harmonigraph_scene::Pulse::Off, 0.0);
    let period = steady_scene.shimmer_width / steady_scene.shimmer_speed;
    let steady = gpu.shot(&steady_scene);
    let swept = (0..8)
        .map(|k| gpu.shot(&at(harmonigraph_scene::Pulse::Bands, period as f64 * k as f64 / 8.0)))
        .collect();
    (steady, swept)
}

/// The pixels one color's sweep moves, for a reading taken over a single shot.
fn swept_pixels(shot: &Shots) -> Vec<usize> {
    (0..shot.0.len() / 4).filter(|&i| swept(shot, i)).collect()
}

/// The pixels a sweep moves in BOTH shots AND that the two colors draw
/// differently when steady.
///
/// Two filters and not one. The intersection is so the two readings are over
/// one set of pixels and neither is averaged over ground the other never
/// covered. The steady difference is what confines the set to the RINGS, which
/// are the only thing the color argument reaches: the octave slice shimmers
/// too, but it takes its color from `scene.pitch_lut` — the fixture's own
/// synthetic ramp, which neither shot varies — so every slice pixel is
/// byte-identical in both shots and lifts by exactly the same amount in each.
/// Left in, they are a block of guaranteed agreement pulling the two colors'
/// readings together, and there are enough of them to carry the assertions
/// below on their own: the comparison would still pass with the rings' shimmer
/// deleted outright, which is the one thing it exists to catch.
fn lifted_pixels(a: &Shots, b: &Shots) -> Vec<usize> {
    (0..a.0.len() / 4)
        .filter(|&i| a.0[i * 4..i * 4 + 3] != b.0[i * 4..i * 4 + 3])
        .filter(|&i| swept(a, i) && swept(b, i))
        .collect()
}

/// One sweep is worth the same CONTRAST on the pitch ramp's dark end as on its
/// bright one — the ratio between a crest and its trough, which is the currency
/// a texture this fine is seen in.
///
/// The currency is the claim. An added light is near-uniform in the `L*` it
/// ADDS — 21.6 to 22.4 across the ramp here, a 13% spread — which is the
/// property such a sheet is tuned to hold; but the crest-to-trough RATIO
/// under it falls from 0.514 at
/// the ramp's dark end to 0.369 at its bright one, a 28% decline, and with the
/// fresh view's bloom on it is a 35% one. A moving texture is read by that
/// ratio rather than by the difference, which is why the sheet reads weaker on
/// the ramp's bright half however uniform the light it adds. An exposure makes
/// the ratio the constant instead and lets the difference vary — the trade
/// taken deliberately, and the reason `SHIMMER_EXPOSURE` is a gain rather than
/// an amount.
///
/// The bound is a tenth where an added light's could bear no better than a
/// quarter, because this is the property the model HOLDS rather than one it
/// approximates: a multiply
/// is one ratio by construction, and what is left to vary is the layers under
/// the rings that the sheet does not touch. Measured at 3% over the ramp with
/// bloom off and 7% with it on.
///
/// The two colors are the ramp's own ends, injected as the node's ring colors.
/// The table the SHADER samples is the fixture's synthetic ramp and is not what
/// the rings wear — which is exactly why `lifted_pixels` has to drop the pixels
/// that draw the same in both shots. Bloom is off (`parity_scene`'s own
/// setting): a halo is a wide blur added over a fine texture, which raises a
/// pixel's mean without raising its swing, and it lands unevenly along the ramp
/// because the threshold's knee is steepest where the dark end sits. That is a
/// real cost of the post pass and worth its own reading; it is not what the
/// sheet does, which is what this asks.
#[test]
fn the_sweep_is_worth_the_same_contrast_on_a_dark_color_as_on_a_bright_one() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let lut = harmonigraph_scene::pitch_ramp_lut(
        harmonigraph_scene::ViewConfig::default().pitch_gradient,
    );
    let (dark, bright) = (lut[0], lut[harmonigraph_scene::PITCH_LUT_N - 1]);

    let dim = sweep_over_color(&mut gpu, dark);
    let lit = sweep_over_color(&mut gpu, bright);
    let shared = lifted_pixels(&dim, &lit);
    assert!(
        shared.len() > 200,
        "only {} pixels shimmered in both shots — the fixture stopped sweeping the \
         rings and the reading below would be noise",
        shared.len(),
    );

    // Michelson contrast over one cycle, in the light a pixel actually carries:
    // `L*` back through its own curve, so the ratio is a ratio of luminance
    // rather than of a perceptual coordinate that is already a cube root of it.
    let luminance = |l_star: f64| ((l_star + 16.0) / 116.0).powi(3);
    let contrast = |(steady, swept): &(Vec<u8>, Vec<Vec<u8>>)| -> f64 {
        let sum: f64 = shared
            .iter()
            .map(|&i| {
                let base = lightness(&steady[i * 4..i * 4 + 4]);
                let ls = swept.iter().map(|f| lightness(&f[i * 4..i * 4 + 4]));
                let (mut hi, mut lo) = (base, base);
                for l in ls {
                    hi = hi.max(l);
                    lo = lo.min(l);
                }
                let (hi, lo) = (luminance(hi), luminance(lo));
                (hi - lo) / (hi + lo).max(1e-9)
            })
            .sum();
        sum / shared.len() as f64
    };
    let (dim_c, lit_c) = (contrast(&dim), contrast(&lit));
    eprintln!(
        "one cycle is worth contrast {dim_c:.3} on the ramp's dark end, {lit_c:.3} on its \
         bright end"
    );
    // A tenth, against a reading of 3% over the whole ramp. Wider than the
    // measurement by enough for a rasteriser to disagree about a ring edge, and
    // nowhere near wide enough to admit the additive model: an added light
    // reads 28% apart on these same two colors, so the bound separates them
    // several times over, which is what a bound here is for.
    let spread = (dim_c - lit_c).abs() / dim_c.max(lit_c);
    assert!(
        spread < 0.10,
        "one cycle was worth contrast {dim_c:.3} on the ramp's dark end but {lit_c:.3} on \
         its bright end ({:.0}% apart): the sheet is a different size depending on which \
         note it is passing over, which is what one exposure everywhere exists to hold down",
        spread * 100.0,
    );
}

/// Between peaks the layer sits at its own color wherever the ceiling covers
/// the swing: a sweep's trough IS the steady picture rather than a dimmed copy
/// of it, on every color whose luma the swing still fits under
/// `SHIMMER_CEILING` — and where it stops fitting, the standing shade the
/// slide buys is bounded, and grows with how bright the color is.
///
/// This is the half of the model nothing else reads. The contrast test above
/// is indifferent to it — a slid swing is the same crest-to-trough ratio, so
/// that reading passes whether the troughs hold still or the whole ramp rides
/// a standing dimmer — and the chroma and hue test below reads only the crest.
/// A ceiling of 0.5 puts 9 `L*` of standing shade under even the ramp's dark
/// end with every other test in this file green; this is the one that goes
/// red.
///
/// The budgets are the measured shape of that trade, with room for a
/// rasteriser to disagree over ring edges and none for a regression. The slide
/// engages where a color's luma — the shader's dot over the STORED values,
/// not over their decoded light — clears `SHIMMER_CEILING / e^swing`, about
/// 0.40 at this fixture's Intensity of 1. The default ramp crosses that in
/// its upper half: the dark end (luma 0.33) pays nothing and is held to
/// rounding, mid-ramp (0.45) measures 3.7 `L*`, and the bright end (0.64)
/// measures 15 — the encoded-domain slide compounded through the display
/// transfer, and several times what a calibration in decoded light predicts.
/// `SHIMMER_CEILING`'s comment carries the trade; this pins its measured cost
/// so a retune moves a number here rather than a picture only.
#[test]
fn between_peaks_the_layer_sits_at_its_own_color() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let lut = harmonigraph_scene::pitch_ramp_lut(
        harmonigraph_scene::ViewConfig::default().pitch_gradient,
    );
    let ends = [
        ("dark", lut[0]),
        ("mid", lut[harmonigraph_scene::PITCH_LUT_N / 2]),
        ("bright", lut[harmonigraph_scene::PITCH_LUT_N - 1]),
    ];
    let shots = ends.map(|(end, color)| (end, sweep_over_color(&mut gpu, color)));
    // The rings alone. The slice under them wears the fixture's own synthetic
    // ramp, whose brightest entries sit high enough to buy a little slide of
    // their own; the claim here is about the color under test, and the steady
    // difference between the two end colors is what confines a reading to the
    // pixels that wear it — the same fence `lifted_pixels` builds.
    let ring: Vec<usize> = (0..shots[0].1 .0.len() / 4)
        .filter(|&i| shots[0].1 .0[i * 4..i * 4 + 3] != shots[2].1 .0[i * 4..i * 4 + 3])
        .collect();
    for (end, shot) in &shots {
        let moved: Vec<usize> = ring.iter().copied().filter(|&i| swept(shot, i)).collect();
        assert!(
            moved.len() > 200,
            "only {} ring pixels shimmered at the ramp's {end} end — the sweep is not \
             reaching the rings and the trough reading below would be noise",
            moved.len(),
        );
        // How far below its steady self the sweep ever takes a pixel, at the
        // pixel's own darkest moment of the cycle, averaged over the rings.
        let (mut dip, mut base) = (0.0, 0.0);
        for &i in &moved {
            let steady = lightness(&shot.0[i * 4..i * 4 + 4]);
            let low = shot
                .1
                .iter()
                .map(|f| lightness(&f[i * 4..i * 4 + 4]))
                .fold(steady, f64::min);
            dip += steady - low;
            base += steady;
        }
        let (dip, base) = (dip / moved.len() as f64, base / moved.len() as f64);
        eprintln!(
            "the {end} ramp color draws its rings at L* {base:.1}; the sweep's troughs sit \
             {dip:.2} under that"
        );
        let allowed = match *end {
            "dark" => 1.0,
            "mid" => 6.0,
            _ => 17.0,
        };
        assert!(
            dip < allowed,
            "at the ramp's {end} end the sweep holds {dip:.1} L* of standing shade between \
             its peaks (the budget there is {allowed}): the troughs are not the steady \
             layer, which is the promise SHIMMER_CEILING exists to keep",
        );
    }
}

/// A ring keeps its color under a peak — the sweep lights it rather than
/// bleaching it, and lights it rather than turning it some other color.
///
/// HUE is what the sheet holds and chroma is what it spends, and the two bounds
/// below are that trade written down rather than one property measured twice.
///
/// A crest that overflows a channel is desaturated toward the grey of its own
/// light, not clipped. Mixing all three channels toward one value moves them
/// together, so their order survives and the color pales along its own hue; a
/// per-channel clip stops the full channel and lets the others climb past it,
/// which turns the color as it brightens. At Intensity 1 that is the whole
/// difference: 0.7 and 5.0 degrees here against the addition's 0.5 and 15.3.
///
/// The chroma goes the other way and is meant to. The addition keeps 99.6% and
/// 73%; this keeps 88% and 57%, because a uniform sheet wants the ramp's bright
/// end near `L*` 90 and the gamut has almost no chroma to offer that hue up
/// there. The bound is what the trade is allowed to cost, and it sits well
/// clear of the mix toward white this is not — that leaves 15% at every point
/// on the ramp, which is a bleach rather than a highlight.
///
/// BOTH ends, because they spend differently. The dark end has light to give
/// and pays little. The bright end has none and pays most of what is paid,
/// which is where a bound set on the dark end alone would measure nothing.
///
/// Hue as well as chroma, because chroma that survives a rotated hue is a ring
/// that has changed color rather than one that has lit up, and a max-minus-min
/// reading cannot tell those apart. The chroma proxy is the spread between a
/// pixel's channels, which is not a perceptual chroma and does not need to be —
/// it is zero exactly when the color is grey, it moves monotonically with how
/// far from grey the color is, and every shot is read the same way.
#[test]
fn a_ring_keeps_its_color_under_a_sweep_peak() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let lut = harmonigraph_scene::pitch_ramp_lut(
        harmonigraph_scene::ViewConfig::default().pitch_gradient,
    );
    // One pair of bounds for both ends rather than a number dialled per end:
    // the bright end is what they are set from, since it is the end with no
    // room above it, and a per-end figure would let a retune that started
    // bleaching the dark end pass by being compared against itself.
    //
    // Half the chroma, against a reading of 57% at the end that pays; and 8
    // degrees of hue, against 5.0 there, where an addition needs
    // 20 to pass at all. The hue bound is the one doing the work — it sits three
    // times inside the addition's 15.3, so a model that went back to clipping
    // the channels separately fails it on the first peak. The chroma bound is a
    // BUDGET rather than a guarantee: it says the sheet may pale a bright crest
    // and may not bleach one, with the mix toward white's 15% at every point on
    // the ramp as the far side of that line.
    const KEEPS_CHROMA: f64 = 0.5;
    const HUE_SWING: f64 = 8.0;

    let ends = [("dark", lut[0]), ("bright", lut[harmonigraph_scene::PITCH_LUT_N - 1])];
    for (end, color) in ends {
        let shot = sweep_over_color(&mut gpu, color);
        let lit = swept_pixels(&shot);
        assert!(lit.len() > 200, "only {} pixels shimmered at the {end} end", lit.len());

        let chroma = |px: &[u8]| {
            let (r, g, b) = (f64::from(px[0]), f64::from(px[1]), f64::from(px[2]));
            (r.max(g).max(b) - r.min(g).min(b)) / 255.0
        };
        let (steady, frames) = &shot;
        let (mut base_sum, mut peak_sum, mut swing_sum, mut swing_n) = (0.0, 0.0, 0.0, 0usize);
        for &i in &lit {
            let px = &steady[i * 4..i * 4 + 4];
            // The color at the pixel's OWN brightest moment, which is the moment
            // the claim is about — the chroma of some other frame would be a
            // reading of a peak that was somewhere else.
            let at = frames
                .iter()
                .map(|f| &f[i * 4..i * 4 + 4])
                .max_by(|a, b| lightness(a).total_cmp(&lightness(b)))
                .expect("eight frames");
            base_sum += chroma(px);
            peak_sum += chroma(at);
            // Only where both readings have a hue to compare. A pixel the peak
            // drives to grey has no angle, and counting the arbitrary one it
            // reports would read a bleach as a rotation — the chroma bound
            // above is what catches that pixel.
            if let (Some(was), Some(now)) = (hue_degrees(px), hue_degrees(at)) {
                let d = (now - was).abs();
                swing_sum += d.min(360.0 - d);
                swing_n += 1;
            }
        }
        let n = lit.len() as f64;
        let (base, peak) = (base_sum / n, peak_sum / n);
        let swing = swing_sum / swing_n.max(1) as f64;
        eprintln!(
            "{end} end: chroma {base:.3} steady, {peak:.3} at the peak; hue moves {swing:.1} deg"
        );
        assert!(
            peak >= base * KEEPS_CHROMA,
            "at the ramp's {end} end a peak left {peak:.3} of the ring's {base:.3} chroma: \
             the sheet is bleaching the color out rather than paling a crest of it, and \
             the budget for that is half",
        );
        assert!(
            swing < HUE_SWING,
            "at the ramp's {end} end a peak swung the ring's hue by {swing:.1} degrees: \
             the light is turning the color rather than lighting it, which is what \
             lifting the channels that have headroom and not the one that does not does",
        );
    }
}
