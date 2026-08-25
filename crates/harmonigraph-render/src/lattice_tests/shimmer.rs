//! The shimmer: its patterns, the clock it travels on, and how wide a band
//! it lays across a node.

use super::fixtures::*;
use crate::*;

/// Every pattern in the row draws its OWN picture — pairwise, at one instant.
///
/// `pulse_marks` is Off in `parity_scene` and every fixture derived from it
/// (deliberately — see that scene's own comment), so nothing else in this file
/// takes a `mode != 0u` branch in the shader: each arm of `shimmer_pattern` is
/// validated by `baked_shader_validates` (parsed, never run) but not otherwise
/// exercised by any render. This runs all of them.
///
/// Pairwise rather than each-against-Off, because "it changed something" is
/// the weaker half of the claim and the one an accident passes: two patterns
/// that fell through to the same arm of the shader, or a mode index off by one
/// anywhere along `Pulse::shader_index` -> misc6.w -> `shimmer_pattern`, would
/// each differ from Off perfectly well while being the same picture as each
/// other. The row is only a row if its options are distinguishable.
///
/// It is a single INSTANT for the same reason: two patterns compared across
/// their own frames would differ merely by having moved.
#[test]
fn every_shimmer_pattern_draws_a_different_picture() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    use harmonigraph_scene::Pulse;

    // Off first, so the loop below has the steady picture to measure against
    // as well as the other patterns.
    let modes = [Pulse::Off, Pulse::Bands, Pulse::Checker, Pulse::Hex];
    // One node with an end marked: the sheet needs a ring to belong to.
    let shots: Vec<(Pulse, Vec<u8>)> = modes
        .iter()
        .map(|&mode| {
            let mut scene = single_marked_node(MIDDLE_C, 0);
            scene.pulse_marks = mode;
            scene.now = 0.4;
            (mode, gpu.shot(&scene))
        })
        .collect();

    for (i, (mode, px)) in shots.iter().enumerate() {
        for (other, other_px) in &shots[i + 1..] {
            assert!(
                differing_pixels(px, other_px) > 0,
                "{mode:?} and {other:?} drew the same picture at the same instant; \
                 they are one option wearing two labels",
            );
        }
    }
}

/// One period of travel returns two of the three patterns to the picture they
/// drew, and Hex to its opposite.
///
/// This is the shape the shader's periodicity actually has, and
/// `Scene::shimmer_slide` reduces a song position against it — so what the
/// modulus there has to be is measured here rather than reasoned about at the
/// other end of the pipe. Hex crosses three gratings sixty degrees apart and
/// the outer two take the travel through a `cos 60`, which halves their rate
/// along their own axes: it closes a cycle over TWO periods, and reducing a
/// clock by one would land it on this test's second assertion, silently, at
/// every wrap.
///
/// Rendered rather than argued because the alternative — asserting that a
/// reduced clock draws what an unreduced one would — cannot be written: the
/// reduction is what produces the number the shader sees, so both sides of
/// that comparison are the same uniform. Turning it around is what makes it
/// observable, and this is the turned-around form.
#[test]
fn one_period_of_travel_repeats_every_pattern_but_hex() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    use harmonigraph_scene::Pulse;
    // The pair `the_mark_sheet_reaches_the_slice_whole` sweeps at: a period
    // well inside what this size resolves, at a pace that crosses it.
    const WIDTH: f32 = 1.2;
    const SPEED: f32 = 1.6;
    let at = |mode: Pulse, now: f64| -> Scene {
        let mut scene = single_marked_node(MIDDLE_C, 0);
        scene.pulse_marks = mode;
        scene.shimmer_width = WIDTH;
        scene.shimmer_speed = SPEED;
        scene.now = now;
        scene
    };
    // Seconds of clock that carry the sheet one period along.
    let period = (WIDTH / SPEED) as f64;
    // Off zero, so a pattern that ignored the clock entirely would not pass
    // the first half by drawing its rest state twice.
    const BASE: f64 = 0.3;

    for mode in [Pulse::Bands, Pulse::Checker] {
        let (before, after) = (gpu.shot(&at(mode, BASE)), gpu.shot(&at(mode, BASE + period)));
        // Half a percent rather than byte-equality, though it measures zero
        // here: the two shots reach the same phase by different arithmetic —
        // one is the other's plus a period, through a sine whose argument is
        // scaled by a reciprocal — so a driver rounding one of them the other
        // way is a byte, not a defect. What this guards against redraws the
        // sheet, not a byte of it.
        let moved = differing_pixels(&before, &after);
        assert!(
            moved * 200 < before.len(),
            "{mode:?} redrew {moved} pixels a period of travel later; it takes the sheet's \
             own period whole, so a period of travel is where it repeats",
        );
    }

    let (before, after) =
        (gpu.shot(&at(Pulse::Hex, BASE)), gpu.shot(&at(Pulse::Hex, BASE + period)));
    assert!(
        differing_pixels(&before, &after) > 0,
        "Hex drew the same picture a period of travel later, so its cycle is one period \
         and not two — and `Scene::shimmer_slide` is reducing the clock by twice what it \
         has to, or this pattern's gratings have moved off sixty degrees",
    );
}

/// The Softness bar reaches the picture, and it is the SHAPE it moves rather
/// than the amount of light.
///
/// Held still (speed 0) and at one instant, so what is compared is two
/// profiles of the same sheet in the same place rather than two moments of
/// one. Three claims, and it takes all three to say "shape":
///
/// - the two ends draw differently, which is the bar working at all;
/// - the gradual end lights MORE over the layer as a whole, the fall from the
///   peak taking most of the period instead of a narrow crest;
/// - and it does that without going any BRIGHTER at its brightest. That last
///   is what rules out the wiring this could otherwise have — a bar on
///   `SHIMMER_EXPOSURE`, raising the peak rather than widening the fall from
///   it, passes the first two and fails this one. The peak is Intensity's to
///   move, and the shape's own crest is pinned wherever it lands: `pow(1, n)`
///   is 1 for every exponent, so however the profile is dialled the brightest
///   pixel is the same pixel at the same value.
#[test]
fn shimmer_softness_spreads_the_light_without_raising_the_peak() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };

    // Both ends marked, so the sheet has two rings and the slice they name to
    // fall across: the light this measures is all of what a sweep puts on the
    // picture, and one ring's worth of it would be a thinner reading of the
    // same claim.
    let at = |softness: f32| -> Scene {
        let mut scene = single_marked_node(MIDDLE_C, MIDDLE_C);
        scene.pulse_marks = harmonigraph_scene::Pulse::Bands;
        scene.shimmer_softness = softness;
        scene.shimmer_speed = 0.0;
        scene.now = 0.4;
        scene
    };
    let light =
        |px: &[u8]| -> u64 { px.chunks(4).map(|p| p[0] as u64 + p[1] as u64 + p[2] as u64).sum() };

    let crisp = gpu.shot(&at(0.0));
    let gradual = gpu.shot(&at(1.0));
    assert!(
        differing_pixels(&crisp, &gradual) > 0,
        "the Softness bar did not reach the picture at all",
    );
    let (crisp_light, gradual_light) = (light(&crisp), light(&gradual));
    assert!(
        gradual_light > crisp_light,
        "the gradual end lit {gradual_light} against the crisp end's {crisp_light}: \
         softening the profile is supposed to spread the light over more of the \
         period, not to dim it",
    );

    // The brightest pixel THE SHEET REACHES, which is the mask below rather
    // than the whole frame: the brightest pixel in the frame is the core disc,
    // which no sheet touches, and a peak read there would come out the same
    // number at both ends however the bar were wired -- the one claim this
    // test exists for, passing vacuously.
    let steady = {
        let mut scene = at(0.0);
        scene.pulse_marks = harmonigraph_scene::Pulse::Off;
        gpu.shot(&scene)
    };
    let swept: Vec<usize> = (0..steady.len() / 4)
        .filter(|&i| {
            let px = i * 4..i * 4 + 4;
            steady[px.clone()] != crisp[px.clone()] || steady[px.clone()] != gradual[px]
        })
        .collect();
    assert!(!swept.is_empty(), "neither end of the bar swept a single pixel");
    let peak = |img: &[u8]| -> u32 {
        swept
            .iter()
            .map(|&i| img[i * 4] as u32 + img[i * 4 + 1] as u32 + img[i * 4 + 2] as u32)
            .max()
            .unwrap_or(0)
    };

    // A hair of tolerance, and only for the rounding two paths through the
    // same arithmetic can land either side of: the claim is that the crest
    // does not MOVE, which a peak-wired bar would break by whole channel
    // steps. Measured dead equal at both ends.
    let (crisp_peak, gradual_peak) = (peak(&crisp), peak(&gradual));
    eprintln!("brightest pixel: {crisp_peak} crisp, {gradual_peak} gradual");
    assert!(
        gradual_peak <= crisp_peak + 2,
        "the gradual end's brightest pixel is {gradual_peak} against the crisp \
         end's {crisp_peak}: Softness is raising the peak rather than widening the \
         fall from it, which is Intensity's job and not this bar's",
    );
}

/// A sheet must draw differently from Off and must move with the clock.
///
/// Off must ALSO be steady across the clock, which is the half that keeps the
/// rest honest: a picture that moved with time in every mode would pass the
/// two "it changed" claims below without the sheet doing anything. It is
/// checked on a node with NO mark at all, which is also the containment claim
/// `the_mark_shimmer_reaches_the_octave_slice_it_points_at` makes in full:
/// nothing about an unmarked node depends on the clock.
///
/// The instants are picked without reference to the fixture's own speed or
/// width: the claim is that the clock reaches the layer, not that a
/// particular phase does, so retuning the sweep cannot make this pass by
/// accident. (Which is also why this one does NOT read
/// [`PARITY_SHIMMER_WIDTH`] the way `SHIMMER_PROBE_STEP` has to — a probe
/// that asks WHERE the bands are needs sizing against them; one that asks
/// whether they move at all does not.)
#[test]
fn the_mark_shimmer_sweeps_the_rings_and_moves_with_time() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };

    let mut off = single_marked_node(0, 0);
    off.now = 0.4;
    let off_a = gpu.shot(&off);
    off.now = 1.1;
    let off_b = gpu.shot(&off);
    assert_eq!(differing_pixels(&off_a, &off_b), 0, "Pulse::Off must not depend on scene.time");

    // The rings need a mark to exist at all -- that is the ring, not the
    // shimmer -- so this marks one end, and the steady shot of the same
    // fixture is what isolates what `pulse_marks` did.
    let mut ring_off = single_marked_node(MIDDLE_C, 0);
    ring_off.now = 0.4;
    let ring_off_a = gpu.shot(&ring_off);

    let mut marks = single_marked_node(MIDDLE_C, 0);
    marks.pulse_marks = harmonigraph_scene::Pulse::Bands;
    marks.now = 0.4;
    let marks_a = gpu.shot(&marks);
    assert!(
        differing_pixels(&ring_off_a, &marks_a) > 0,
        "the mark rings' sheet drew the steady picture"
    );
    marks.now = 1.1;
    let marks_b = gpu.shot(&marks);
    assert!(
        differing_pixels(&marks_a, &marks_b) > 0,
        "the mark rings' sheet did not change between two different \
         times; it is not reading the clock"
    );
}

/// The sweep's two settings reach the picture, and the clock reaches it only
/// THROUGH the speed.
///
/// The last part is what makes this more than two "something changed"
/// probes. Speed and width both scale the same phase (`travel / period`,
/// with the clock inside `travel`), so a width that had quietly taken the
/// clock's term with it, or a speed read as a frequency in the band count,
/// would still move the picture on both bars and still animate — and would
/// have made the two knobs one. At speed 0 the bands must stand still while
/// the clock runs, whatever the width is set to.
#[test]
fn the_shimmers_speed_and_width_reach_the_picture_and_only_speed_carries_the_clock() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    // Both ends marked, so the sheet has as much of the picture to fall
    // across as the fixture can give it: this is about the sheet's own shape
    // and pace, not about what it crosses.
    let sweep = |speed: f32, width: f32, time: f64| -> Scene {
        let mut scene = single_marked_node(MIDDLE_C, MIDDLE_C);
        scene.pulse_marks = harmonigraph_scene::Pulse::Bands;
        scene.shimmer_speed = speed;
        scene.shimmer_width = width;
        scene.now = time;
        scene
    };

    // A time well off zero, or the speed would have nothing to multiply.
    let base = gpu.shot(&sweep(1.6, 5.0, 0.4));
    assert!(
        differing_pixels(&base, &gpu.shot(&sweep(3.2, 5.0, 0.4))) > 0,
        "the Speed bar did not move the bands: at a fixed instant it is what \
         says how far along their normal they have travelled",
    );
    assert!(
        differing_pixels(&base, &gpu.shot(&sweep(1.6, 2.5, 0.4))) > 0,
        "the Spacing bar did not resize the bands",
    );

    // Held still: the sheet is where it started at every instant, and stays
    // there through three widths spanning the resolvable half of the bar.
    for width in [2.5, 5.0, 12.0] {
        let (early, late) = (gpu.shot(&sweep(0.0, width, 0.4)), gpu.shot(&sweep(0.0, width, 9.7)));
        assert_eq!(
            differing_pixels(&early, &late),
            0,
            "at speed 0 the bands still moved between two instants at width \
             {width}; the clock is reaching the sweep by some route other than \
             the speed, and the two bars are not the independent pair they read as",
        );
    }
}

/// The mark rings' sweep reaches the slice WHOLE — both halves of a band's
/// swing, not just the bright one.
///
/// A band is an exposure around the layer's own color (`shimmer_light`): the
/// whole swing runs upward where the ceiling leaves it room, and slides below
/// the layer's color where the swing outruns that room — which this fixture's
/// ring colors do, so the sheet here has a dark half, and the dark half is
/// what gives it a body to travel through. The ring takes both. The slice that
/// ring names has to take both as well, or one mark is lit by two different
/// lights: the annulus dipping between bands while the wedge it points at
/// only ever brightens.
///
/// The dip is the half a plausible wiring drops, which is why it is measured
/// rather than assumed. The slice takes the sheet through a SWING scaled by how
/// much of the pixel is a wedge some ring points at, and a wiring that scaled
/// the band's shape instead would leave the slice sitting at a phase rather
/// than at rest — brightening perfectly well while never dipping, and looking
/// right everywhere except in the half nobody checks.
#[test]
fn the_mark_sheet_reaches_the_slice_whole() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    // Tight enough that a band and the gap after it both fall across the
    // node, so one instant of the cycle has the slice in a trough rather
    // than the whole node riding one shoulder of a band five nodes wide.
    const WIDTH: f32 = 1.2;
    const SPEED: f32 = 1.6;
    let scene = |melody: u32, marks: harmonigraph_scene::Pulse, time: f64| -> Scene {
        let mut scene = single_marked_node(melody, 0);
        scene.pulse_marks = marks;
        scene.shimmer_width = WIDTH;
        scene.shimmer_speed = SPEED;
        scene.now = time;
        scene
    };
    let off = harmonigraph_scene::Pulse::Off;
    let shimmer = harmonigraph_scene::Pulse::Bands;

    // One cycle, walked: the trough has to pass over the slice somewhere in
    // it whatever phase the fixture happens to start at.
    let (mut dimmed_slice, mut dimmed_ring) = (0usize, 0usize);
    for step in 0..8 {
        let time = 0.2 + step as f64 * (WIDTH / SPEED) as f64 / 8.0;
        // The rings, from the node that wears none — the same mask
        // `the_mark_shimmer_reaches_the_octave_slice_it_points_at` takes, so
        // what is left is the glyph layer.
        let bare = gpu.shot(&scene(0, off, time));
        let steady = gpu.shot(&scene(MIDDLE_C, off, time));
        let swept = gpu.shot(&scene(MIDDLE_C, shimmer, time));
        for i in 0..swept.len() / 4 {
            let px = i * 4..i * 4 + 4;
            let on_ring = bare[px.clone()] != steady[px.clone()];
            let (before, after) = (brightness(&steady[px.clone()]), brightness(&swept[px]));
            // One count of rounding per channel: the two shots run the same
            // arithmetic to a different answer only where the sheet falls, and
            // a term that lands on a channel boundary can round down.
            if after < before - 3 {
                if on_ring {
                    dimmed_ring += 1;
                } else {
                    dimmed_slice += 1;
                }
            }
        }
    }
    eprintln!("mark sheet dimmed {dimmed_ring} px of ring and {dimmed_slice} px of slice");
    // The control: the sheet HAS a trough at these instants, and it reaches
    // the rings it is laid over. Without this the slice figure below could be
    // zero because nothing was sweeping at all.
    assert!(
        dimmed_ring > 0,
        "the mark rings' shimmer never dimmed a ring pixel across a whole cycle; \
         the sweep has no trough at these instants and the slice claim below is \
         measuring nothing",
    );
    // A floor rather than a share of the ring: how much of the band the
    // fixture shows is a setting, and the slice is one wedge of it against
    // two full annuli. Measured 625 px of slice against 5781 of ring; a
    // wiring that scaled the band's shape rather than its swing reads 0 of
    // slice against that same ring count, brightening at every phase and
    // never dipping.
    assert!(
        dimmed_slice > 200,
        "the mark rings' shimmer dimmed {dimmed_ring} px of ring but only \
         {dimmed_slice} px of the slice those rings point at: the sheet's trough \
         stops at the ring's edge, so the wedge only ever brightens and the one \
         mark is lit by two different lights",
    );
}

/// Intensity is the DEPTH of the sweep, and its bottom end is the identity:
/// at 0 the layer draws exactly as it does with the mode Off, byte for byte.
///
/// That last claim is the one worth pinning: `shimmer_light` is applied
/// unconditionally rather than behind the mode switch, so the bar's bottom
/// has to be the exact identity and not nearly one. A layer coming back a
/// rounding under itself would be a steady dimming with no shimmer in it —
/// on every frame, at a setting that reads as "off".
#[test]
fn shimmer_intensity_scales_the_sweep_and_bottoms_out_at_the_steady_layer() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    // An instant with a band actually over the node: an intensity bar cannot
    // be read at a moment when there is nothing to scale.
    let at = |intensity: f32, pulse: harmonigraph_scene::Pulse| -> Scene {
        let mut scene = single_marked_node(MIDDLE_C, MIDDLE_C);
        scene.pulse_marks = pulse;
        scene.shimmer_intensity = intensity;
        scene.shimmer_speed = 0.0;
        scene.now = 0.4;
        scene
    };
    let off = harmonigraph_scene::Pulse::Off;
    let shimmer = harmonigraph_scene::Pulse::Bands;

    let steady = gpu.shot(&at(1.0, off));
    assert_eq!(
        differing_pixels(&steady, &gpu.shot(&at(0.0, shimmer))),
        0,
        "intensity 0 did not draw the steady layer: the sweep still has one of \
         its two terms running, which at the bottom of the bar is a standing \
         dimming rather than a shimmer",
    );

    let full = gpu.shot(&at(1.0, shimmer));
    let half = gpu.shot(&at(0.5, shimmer));
    assert!(differing_pixels(&steady, &full) > 0, "intensity 1 drew the steady layer");
    assert!(
        differing_pixels(&half, &full) > 0 && differing_pixels(&half, &steady) > 0,
        "half intensity is indistinguishable from full or from off; the bar is \
         a switch rather than a depth",
    );
    // ...and it is a depth in the direction the name says: a deeper setting
    // takes the picture FURTHER from its steady self.
    //
    // Departure rather than added light, because an exposure fitted to the
    // layer's headroom does not promise light in both directions and should not
    // be asked for it. Where a color has room the sheet adds light; where it has
    // none — this fixture's ring colors sit at the top of a channel, the ramp's
    // do not — the same setting is the same RATIO taken as shade instead, so
    // total light falls with intensity there while the sweep gets deeper. That
    // is the bar working, and a total-light reading would call it broken.
    let departure = |shot: &[u8]| -> f64 {
        shot.chunks(4).zip(steady.chunks(4)).map(|(a, b)| (lightness(a) - lightness(b)).abs()).sum()
    };
    let (dim, mid, deep) = (departure(&steady), departure(&half), departure(&full));
    eprintln!("the sweep departs from steady by {dim:.0}/{mid:.0}/{deep:.0} at intensity 0/0.5/1");
    assert!(
        dim < mid && mid < deep,
        "the sweep did not deepen with intensity ({dim:.0}, {mid:.0}, {deep:.0}): a band \
         over the node has to move it further the deeper the sweep is",
    );
}

/// The tight end of the Spacing bar puts SEVERAL bands across one node at once
/// — a texture on the node rather than a sheet passing between nodes — which
/// is a different picture from the wide end and not just a smaller number.
///
/// Counted rather than eyeballed, from a profile taken along the bands' own
/// normal. Each pixel's shimmer is read as the RATIO of its light to the same
/// pixel's light with the mode Off, which cancels everything the node draws —
/// the gaps between sectors, the rings, the glow falling off — and leaves the
/// sweep alone. A band edge is where that ratio crosses the profile's OWN mean,
/// so counting crossings counts band edges, with no threshold picked to suit
/// the answer.
///
/// The mean and not 1. The sheet is an exposure fitted to each layer's
/// headroom, so where a color has no room above it — this fixture's ring
/// colors sit at the top of a channel —
/// the whole sweep is shade and the ratio never reaches 1 at all. Counting
/// crossings of 1 there finds a profile entirely on one side of the line and
/// reports no bands in a picture full of them. The mean rides wherever the
/// sweep put the profile and asks the question the test is actually about,
/// which is how many bands fit across one node.
///
/// A deadband either side of the mean keeps a bin that is merely quiet from
/// reading as a crossing; bins with little paint in them are dropped outright.
#[test]
fn a_tight_width_puts_several_bands_across_one_node() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let sweeping = |width: f32| -> Scene {
        // Both ends marked: the rings are two full annuli spanning the node,
        // so a profile taken across them samples the whole diameter the bands
        // have to fit into rather than one wedge of it.
        let mut scene = single_marked_node(MIDDLE_C, MIDDLE_C);
        scene.pulse_marks = harmonigraph_scene::Pulse::Bands;
        scene.shimmer_width = width;
        // Held still, so the profile is one instant of the sheet and not a
        // smear of where it was going.
        scene.shimmer_speed = 0.0;
        scene.now = 0.4;
        scene
    };
    let steady = {
        let mut scene = sweeping(5.0);
        scene.pulse_marks = harmonigraph_scene::Pulse::Off;
        gpu.shot(&scene)
    };

    // The bands run SHIMMER_ANGLE_TURNS (three eighths of a turn) off the
    // camera's right axis, toward its up axis — which is up the screen,
    // against the row index running down it. That direction is left-and-up, so
    // a fragment's place along the bands' normal is x + y, and binning by it
    // is binning by band phase. Four pixels to a bin: fine enough to resolve
    // the tightest width the bar reaches at this node size, coarse enough that
    // a bin holds a real sample.
    let mut bands_crossing = |width: f32| -> usize {
        let swept = gpu.shot(&sweeping(width));
        let bins = 2 * SIZE[0] as usize / 4;
        let mut lit = vec![0i64; bins];
        let mut here = vec![0i64; bins];
        for (i, (a, b)) in steady.chunks(4).zip(swept.chunks(4)).enumerate() {
            let (x, y) = (i % SIZE[0] as usize, i / SIZE[0] as usize);
            let bin = (x + y) / 4;
            lit[bin] += a[0] as i64 + a[1] as i64 + a[2] as i64;
            here[bin] += b[0] as i64 + b[1] as i64 + b[2] as i64;
        }
        // A bin needs real paint in it before its ratio means anything; the
        // node covers a fraction of the frame, and the empty ground either
        // side of it would otherwise contribute a ratio of 0/0.
        let floor = lit.iter().max().copied().unwrap_or(0) / 8;
        let ratios: Vec<f64> = lit
            .iter()
            .zip(&here)
            .filter(|(l, _)| **l >= floor.max(1))
            .map(|(l, h)| *h as f64 / *l as f64)
            .collect();
        let mean = ratios.iter().sum::<f64>() / ratios.len().max(1) as f64;
        let mut crossings = 0;
        let mut above: Option<bool> = None;
        for (l, h) in lit.iter().zip(&here) {
            if *l < floor.max(1) {
                continue;
            }
            let ratio = *h as f64 / *l as f64;
            let now = if ratio > mean * 1.01 {
                Some(true)
            } else if ratio < mean * 0.99 {
                Some(false)
            } else {
                None
            };
            if let (Some(was), Some(is)) = (above, now) {
                if was != is {
                    crossings += 1;
                }
            }
            above = now.or(above);
        }
        crossings
    };

    // At the WIDE end of the bar the node is a fraction of one band, so its
    // whole paint sits on one side of the sweep or slides across at most one
    // edge of it. (Not the fresh view's width, which sits down near the tight
    // end — see `PARITY_SHIMMER_WIDTH`.)
    let wide = bands_crossing(PARITY_SHIMMER_WIDTH);
    // At a tight one the node is several bands across. The fixture's node is
    // 1.1 world units in radius against a width of 0.35, so the octave band
    // alone spans about five. Measured 1 edge wide against 21 tight.
    let tight = bands_crossing(0.35);
    eprintln!("band edges across the node: {wide} wide, {tight} tight");
    assert!(
        wide <= 2,
        "the wide end already puts {wide} band edges across one node; it is \
         supposed to be a sheet crossing the lattice, and the two ends of the \
         bar are then the same picture",
    );
    assert!(
        tight >= 6,
        "a width of 0.35 put only {tight} band edges across the node (the wide \
         end puts {wide}); the tight end of the bar is not reaching the \
         several-bands-per-node look it exists for",
    );
}

/// Past the tight end the sheet runs out of PIXELS to be drawn in, and what
/// it does there is fade out rather than alias.
///
/// A sine sampled once per fragment stops meaning anything at half a period
/// to the pixel: past that the pattern does not get finer, it turns into a
/// moire of the sampling grid, which crawls as the camera moves and lands
/// differently at every render size — the one thing the sweep's world units
/// exist to avoid. `shimmer_terms` fades the depth out over
/// SHIMMER_RESOLVE_FULL..GONE instead, so the layer settles back onto exactly
/// the picture Off draws.
///
/// The two claims are a pair and neither means much alone: a width the frame
/// CAN resolve has to still sweep (or the fade is just a broken sheet), and
/// the finest width the bar reaches has to be pixel-identical to Off (or the
/// fade stops somewhere short of the identity and leaves a haze that no
/// setting can clear).
#[test]
fn a_width_finer_than_the_pixels_fades_out_instead_of_aliasing() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };

    let at = |width: f32| -> Scene {
        let mut scene = single_marked_node(MIDDLE_C, MIDDLE_C);
        scene.pulse_marks = harmonigraph_scene::Pulse::Bands;
        scene.shimmer_width = width;
        // Held still: a fade measured across two instants would be measuring
        // the travel as well.
        scene.shimmer_speed = 0.0;
        scene.now = 0.4;
        scene
    };
    let steady = {
        let mut scene = at(0.35);
        scene.pulse_marks = harmonigraph_scene::Pulse::Off;
        gpu.shot(&scene)
    };

    // The same width the test above counts fifteen band edges at, which this
    // node's pixels carry comfortably.
    let resolvable = differing_pixels(&steady, &gpu.shot(&at(0.35)));
    // The floor `derive_scene` clamps the Spacing bar to.
    let finest = differing_pixels(&steady, &gpu.shot(&at(0.02)));
    eprintln!("pixels swept: {resolvable} at a resolvable width, {finest} at the floor");
    assert!(
        resolvable > 0,
        "a width the frame can resolve swept nothing; the fade is eating the sheet \
         well before the pixels run out",
    );
    assert_eq!(
        finest, 0,
        "the finest width the bar reaches still moved {finest} px against the steady \
         layer, so what it is drawing there is a moire of the pixel grid rather \
         than the picture Off draws",
    );
}

/// The mark rings' shimmer also sweeps the octave SLICE each ring points at,
/// which is drawn by the glyph layer — a mark is the ring together with the
/// octave it names, and light crossing the one has to cross the other or it
/// cuts the mark in half at the gap between them.
///
/// The claim is about paint OUTSIDE the rings, so the rings are masked off
/// rather than switched off: `mark_thickness = 0` would take the rings and
/// the slice sweep with them (`the_mark_pulse_folds_off_when_the_rings_are_off`
/// in harmonigraph-scene folds the mode there, and a fixture the app cannot
/// build is not a reading of what it draws). The mask is measured instead —
/// an unmarked node wears no rings, so the pixels a marked one differs from
/// it at ARE the rings, fringe and all, whatever radii the band setting put
/// them at. What is left is the rest of the node, where only the glyph layer
/// draws.
#[test]
fn the_mark_shimmer_reaches_the_octave_slice_it_points_at() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    // One instant, held across all four shots: the sweep moves, so anything
    // compared across two clocks would differ whatever it drew.
    let at = |melody: u32, pulse: harmonigraph_scene::Pulse| -> Scene {
        let mut scene = single_marked_node(melody, 0);
        scene.now = 0.4;
        scene.pulse_marks = pulse;
        scene
    };
    let off = harmonigraph_scene::Pulse::Off;
    let shimmer = harmonigraph_scene::Pulse::Bands;

    // No mark: no ring to sweep and no slice to reach, so the mode changes
    // nothing at all. This is the containment half of the claim — the mark
    // layer's sweep must not have become a second octave-layer sweep.
    let bare = gpu.shot(&at(0, off));
    let bare_shimmer = gpu.shot(&at(0, shimmer));
    assert_eq!(
        differing_pixels(&bare, &bare_shimmer),
        0,
        "an unmarked node changed under the mark rings' shimmer; the sweep has \
         escaped the slices a ring points at and is crossing the whole octave layer",
    );

    let steady = gpu.shot(&at(MIDDLE_C, off));
    let swept = gpu.shot(&at(MIDDLE_C, shimmer));
    // Where the rings draw, from the node that wears none.
    let ring = |i: usize| bare[i * 4..i * 4 + 4] != steady[i * 4..i * 4 + 4];
    let (mut on_ring, mut past_ring) = (0usize, 0usize);
    for i in 0..steady.len() / 4 {
        if steady[i * 4..i * 4 + 4] == swept[i * 4..i * 4 + 4] {
            continue;
        }
        if ring(i) {
            on_ring += 1;
        } else {
            past_ring += 1;
        }
    }
    eprintln!("mark shimmer moved {on_ring} px of ring and {past_ring} px past it");
    // A floor rather than a share of the rings: the slice is one wedge of the
    // band against two full annuli, and how much of the band the fixture
    // shows is a setting. Measured 599 px past the ring, against 940 on it.
    assert!(
        past_ring > 200,
        "the mark rings' shimmer moved only {past_ring} px outside the rings \
         ({on_ring} on them): it is sweeping the annulus alone and stopping at \
         the gap, leaving the octave slice the mark names unlit",
    );
}

/// Mirrors `SHIMMER_ANGLE` in lattice.wgsl, as a fraction of a turn from the
/// camera's right axis toward its up axis — the direction the bands travel.
///
/// Held to the shader's own literal by `the_probe_moves_along_the_angle_the_shader_sweeps`
/// rather than by a comment asking for it: the probe below moves a node across
/// this direction and along it and reads how much each move costs, so an angle
/// that drifted from the shader's would leave the test comparing two arbitrary
/// directions — passing on its margin while measuring nothing about the sheet.
const SHIMMER_ANGLE_TURNS: f32 = 0.375;

/// The mirror above, enforced. `SHIMMER_ANGLE` is a tuning knob for the look —
/// which diagonal the light rakes across — and retuning it is exactly the edit
/// that would strand the probe.
#[test]
fn the_probe_moves_along_the_angle_the_shader_sweeps() {
    let needle = format!("const SHIMMER_ANGLE: f32 = {SHIMMER_ANGLE_TURNS} * TAU;");
    assert!(
        SHADER_SRC.contains(&needle),
        "lattice.wgsl must declare `{needle}` to match SHIMMER_ANGLE_TURNS; the probe in \
         the_shimmer_is_one_field_across_the_lattice moves nodes across that angle and \
         along it, and against a different one it measures neither",
    );
}

/// `scene`'s only node, moved [`SHIMMER_PROBE_STEP`] world units along the
/// camera-plane direction `turns` of a turn from the camera's right axis.
fn move_node_across_the_view(scene: &mut Scene, turns: f32) {
    let (right, up) = scene.camera.right_up();
    let a = turns * std::f32::consts::TAU;
    scene.nodes[0].world_pos = (right * a.cos() + up * a.sin()) * SHIMMER_PROBE_STEP;
}

/// The sheet is ONE field across the lattice, not a copy per node — the claim
/// that is the whole point of the shimmer, and that the tests above would pass
/// without.
///
/// The field is the fragment's place on the plane the billboards face, so a
/// node MOVED across that plane meets the bands at a different phase and draws
/// with a different amount of light in it. Read off a per-node coordinate
/// (`in.uv`, say) every node would run an identical private copy, moving one
/// would change nothing but where it landed, and the "across" measurement
/// below would collapse into its control.
///
/// The control for that is the SAME move made along the bands instead —
/// perpendicular to the direction they travel, which slides the node down a
/// line the field is constant on and so leaves the picture the one it was.
/// The two directions are mirror images across the camera's up axis, so the
/// two moves put the node in exactly mirrored places: whatever the move costs
/// in rasterization and perspective, it costs both equally, and what is left
/// between them is the shimmer.
#[test]
fn the_shimmer_is_one_field_across_the_lattice() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    // All the light in the frame. A total rather than a pixel-by-pixel
    // count, because a moved node lands in different pixels by design: what
    // is being compared is how much of it the shimmer let through, not
    // where it went.
    // How much the picture's total light changes when `make`'s node moves
    // `turns` of a turn off the camera's right axis, against leaving it at
    // the origin.
    let mut move_cost = |make: &dyn Fn() -> Scene, turns: f32| -> i64 {
        let still = total_light(&gpu.shot(&make()));
        let mut moved = make();
        move_node_across_the_view(&mut moved, turns);
        (total_light(&gpu.shot(&moved)) - still).abs()
    };

    let across_the_bands = SHIMMER_ANGLE_TURNS;
    let along_the_bands = SHIMMER_ANGLE_TURNS + 0.25;

    // The control: with nothing shimmering, a move costs only what moving
    // costs — a node landing on its own pixel grid, and the perspective at
    // a place that is not the middle of the frame.
    let steady = || {
        let mut scene = single_marked_node(MIDDLE_C, MIDDLE_C);
        scene.now = 0.4;
        scene
    };
    let steady_across = move_cost(&steady, across_the_bands);
    let steady_along = move_cost(&steady, along_the_bands);

    let marks = || {
        let mut scene = steady();
        scene.pulse_marks = harmonigraph_scene::Pulse::Bands;
        scene
    };
    let mark_across = move_cost(&marks, across_the_bands);
    let mark_along = move_cost(&marks, along_the_bands);

    eprintln!(
        "steady {steady_across}/{steady_along}, marks {mark_across}/{mark_along} (across/along)"
    );
    // The control has to STAY small, or the ratio below stops being about the
    // shimmer. Should a bare node move ever get expensive or lopsided — a new
    // depth-dependent layer, a cull edge inside the probe's reach, anything
    // keyed on world position — both figures would inflate off the same base,
    // the ratio would collapse, and the failure would be reported as a shimmer
    // defect it is not. Measured 83/110 against the sheet's 9930.
    let steady = steady_across.max(steady_along);
    assert!(
        steady * 10 < mark_across,
        "moving a node costs {steady} even with nothing shimmering, which is too near \
         what the shimmering layer costs ({mark_across}) for the difference between \
         them to be the shimmer's"
    );
    // A multiple, not a threshold: the claim is that crossing the bands
    // dominates sliding along them, and the along-figure is the same move
    // mirrored, so it carries the layer's own share of the control above.
    assert!(
        mark_across > mark_along * 4,
        "moving a node across the bands ({mark_across}) barely beat moving it \
         along them ({mark_along}; the steady control costs \
         {steady_across}/{steady_along}) -- either the field is per-node rather \
         than one sheet over the lattice, or the bands are not running the way \
         SHIMMER_ANGLE says"
    );
}
