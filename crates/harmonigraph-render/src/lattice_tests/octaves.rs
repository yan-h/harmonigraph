//! The octave wheel: which slices draw, where each one sits, and what a
//! released one fades to.

use super::fixtures::*;
use crate::gpu_harness::{headless_device, readback, render_to_texture};
use crate::*;

/// The FOLD reading fills the same wedges of the same annulus, and reads each
/// of them at its own octave's PITCH: a wedge is flat, so nothing about the
/// picture depends on Range and a detuned partial dims rather than moving.
///
/// The pair of claims that make the two readings one control over one
/// indicator rather than two features. Both are things the raw reading does
/// the other way — its wedge is a window, so Range zooms it and a detuning
/// slides across it — and a fold that quietly kept the window would pass
/// every geometric claim above while drawing the wrong picture.
#[test]
fn the_folded_ring_reads_each_wedge_at_its_own_octave() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    const UP: usize = harmonigraph_scene::MIDDLE_C_SLOT;
    let slot_pitch = |slot: usize| slot as f32 * 12.0;
    let fresh_range = PROBE_RANGE;
    // The same fixture as the raw reading's, with the wedges read at their own
    // pitch instead of across a window.
    let folded = |sounding: Option<f32>, range: f32| {
        let mut scene = ringing_node(None, sounding, range);
        scene.spectral.folded = true;
        scene
    };

    let base = gpu.shot(&folded(None, fresh_range));
    let raw_base = gpu.shot(&ringing_node(None, None, fresh_range));

    // It draws, in the same annulus and at the same angle the octave's own
    // wedge stands at — the raw reading's own claim, so the two are one ring.
    let on = {
        let shot = gpu.shot(&folded(Some(slot_pitch(UP)), fresh_range));
        light_about_center(&light_over(&shot, &base), SIZE)
    };
    assert!(on.weight > 0.0, "the folded ring drew nothing at all");
    let raw = {
        let shot = gpu.shot(&ringing_node(None, Some(slot_pitch(UP)), fresh_range));
        light_about_center(&light_over(&shot, &raw_base), SIZE)
    };
    let apart = angle_apart(on.angle, raw.angle);
    assert!(apart < 6.0, "the folded wedge sits {apart:.1}° off the raw one for the same octave");
    assert!(
        (on.near - raw.near).abs() < 3.0 && (on.far - raw.far).abs() < 3.0,
        "the folded ring runs {:.1}..{:.1} px against the raw one's {:.1}..{:.1}",
        on.near,
        on.far,
        raw.near,
        raw.far,
    );

    // Range is inert: a wedge is one reading, so there is no window for it to
    // zoom. Pixel-exact, since the shader does not read the setting at all
    // down this branch.
    let narrow = gpu.shot(&folded(Some(slot_pitch(UP)), 50.0));
    let wide = gpu.shot(&folded(Some(slot_pitch(UP)), 1200.0));
    assert_eq!(
        differing_pixels(&narrow, &wide),
        0,
        "Range changed the folded ring, which has no window to size",
    );

    // ...and a detuned partial DIMS where the raw reading would slide it
    // across the wedge: half the fresh window sharp is a quarter-wedge move
    // there and no move at all here. The fixture's partial is a rectangle
    // PARTIAL_HALF_CENTS wide, so at half a window off it has left the
    // octave's own pitch entirely and the wedge goes dark.
    let off_pitch = {
        let shot = gpu.shot(&folded(Some(slot_pitch(UP) + fresh_range / 200.0), fresh_range));
        light_about_center(&light_over(&shot, &base), SIZE)
    };
    eprintln!("on {:.0}, half a window off {:.0}", on.weight, off_pitch.weight);
    assert!(
        off_pitch.weight < 0.25 * on.weight,
        "a partial half a window off still lit the wedge at {:.0} against {:.0} on pitch",
        off_pitch.weight,
        on.weight,
    );
}

/// A node showing every octave it can, for reading the wheel's geometry off
/// the picture: no audio ring and no mark rings, so the only thing drawn is the
/// band, and a wide gap so the seams between indicators are several pixels
/// across at the size this renders at.
fn octave_wheel_scene(layout: harmonigraph_scene::OctaveLayout, cents: f32) -> Scene {
    let mut scene = single_marked_node(0, 0);
    scene.octave_layout = layout;
    scene.outer_inner = 0.30;
    scene.outer_outer = 0.95;
    scene.rings_outer = 0.95;
    scene.octave_gap = 0.10;
    scene.mark_thickness = 0.0;
    // Every octave the wheel draws for THIS pitch class, and only those: a
    // level on a slot no sector draws is a state `derive_scene` cannot reach,
    // and the glow would still take a color from it. Slots outside the packing
    // are what a ring near the pitch limits reaches for, and no note can light
    // one.
    let (low, high) = layout.slots(cents);
    let node = &mut scene.nodes[0];
    node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
    for slot in low.max(0)..=high.min(harmonigraph_scene::OCTAVE_SLOTS as i32 - 1) {
        node.octaves[slot as usize] = 1.0;
    }
    node.cents = cents;
    scene
}

/// Reads mean colors out of one wedge of a rendered node's octave band.
/// Self-calibrating — it finds the band's radii from a picture that has it
/// lit, rather than reproducing the camera's arithmetic, which would only
/// re-assert it.
struct BandProbe {
    size: [u32; 2],
    inner: f32,
    outer: f32,
}

impl BandProbe {
    /// Calibrated along `angle`, on a shot with the band drawn: the node is
    /// alone at the world origin and the camera looks at it, so the frame's
    /// center is its center.
    fn new(px: &[u8], size: [u32; 2], angle: f32) -> BandProbe {
        let mut probe = BandProbe { size, inner: 0.0, outer: 0.0 };
        let on_band: Vec<f32> = (4..size[0] / 2)
            .map(|r| r as f32)
            .filter(|&r| probe.at(px, r, angle).iter().sum::<f32>() > 24.0)
            .collect();
        assert!(!on_band.is_empty(), "nothing lit along the ray at {angle} rad");
        probe.inner = on_band[0];
        probe.outer = on_band[on_band.len() - 1];
        assert!(
            probe.outer - probe.inner > 8.0,
            "no band to sample: {}..{}",
            probe.inner,
            probe.outer
        );
        probe
    }

    /// One pixel, `r` from the center at `a` radians.
    fn at(&self, px: &[u8], r: f32, a: f32) -> [f32; 3] {
        let c = self.size[0] as f32 / 2.0;
        // Screen y grows downward, so the sample angle is negated.
        let (x, y) = (c + r * a.cos(), c - r * a.sin());
        let i = (y as usize * self.size[0] as usize + x as usize) * 4;
        [px[i] as f32, px[i + 1] as f32, px[i + 2] as f32]
    }

    /// Mean color well inside the wedge `width` wide centered on `angle`, in
    /// both directions, so neither the slice's antialiased edges nor the
    /// band's enter the reading.
    fn mean(&self, px: &[u8], angle: f32, width: f32) -> [f32; 3] {
        let (mut sum, mut n) = ([0f32; 3], 0f32);
        let margin = 0.2 * (self.outer - self.inner);
        let mut r = self.inner + margin;
        while r <= self.outer - margin {
            for k in -6..=6 {
                let sample = self.at(px, r, angle + 0.03 * k as f32 * width);
                for j in 0..3 {
                    sum[j] += sample[j];
                }
                n += 1.0;
            }
            r += 1.0;
        }
        sum.map(|s| s / n)
    }
}

/// The middle of slot `slot`'s wedge on a node whose pitch class is `cents`,
/// and how wide it is — the pair [`BandProbe::mean`] samples inside.
fn wedge_of(layout: harmonigraph_scene::OctaveLayout, slot: usize, cents: f32) -> (f32, f32) {
    let (e0, e1) = layout.sector(slot as i32, cents);
    (0.5 * (e0 + e1), e0 - e1)
}

/// The band's lit/unlit profile around a rendered node: index `i` is the
/// angle `360 * i / STEPS` counter-clockwise from screen right.
/// Self-calibrating — it finds the node's center and the band's radius from
/// the image rather than reproducing the camera's arithmetic, which would
/// only re-assert it.
const PROFILE_STEPS: usize = 720;

fn band_profile(px: &[u8], size: u32) -> Vec<bool> {
    let w = size as usize;
    let lit = |x: f32, y: f32| -> bool {
        if x < 0.0 || y < 0.0 || x >= size as f32 || y >= size as f32 {
            return false;
        }
        let i = (y as usize * w + x as usize) * 4;
        px[i] as u32 + px[i + 1] as u32 + px[i + 2] as u32 > 24
    };
    // The node is alone at the world origin and the camera looks at it, so
    // the frame's center is its center. Not the lit pixels' centroid: a
    // fringed band is heavier on the side its wide octaves fall, which would
    // pull a centroid off-center by roughly what the measurement below is
    // trying to see.
    let drawn = (0..size * size).filter(|k| lit((k % size) as f32, (k / size) as f32)).count();
    assert!(drawn > 100, "nothing drawn to measure ({drawn} lit px)");
    let (cx, cy) = (size as f32 / 2.0, size as f32 / 2.0);

    // Sample at whichever radius has the most band on it: picking one by
    // arithmetic would land in a seam or off the band as the settings move.
    // Screen y grows downward, so the sample angle is negated.
    let ring = |r: f32| -> Vec<bool> {
        (0..PROFILE_STEPS)
            .map(|i| {
                let a = std::f32::consts::TAU * i as f32 / PROFILE_STEPS as f32;
                lit(cx + r * a.cos(), cy - r * a.sin())
            })
            .collect()
    };
    let best = (4..size / 2)
        .map(|r| (ring(r as f32).iter().filter(|b| **b).count(), r))
        .max()
        .expect("a band to sample")
        .1;
    ring(best as f32)
}

/// Width in degrees of the unlit run containing `at_degrees`, or 0 if that
/// direction is lit.
fn gap_at(profile: &[bool], at_degrees: f32) -> f32 {
    let step = 360.0 / PROFILE_STEPS as f32;
    let start = (at_degrees / step).round() as usize % PROFILE_STEPS;
    if profile[start] {
        return 0.0;
    }
    let mut run = 1;
    let mut k = 1;
    while k < PROFILE_STEPS && !profile[(start + k) % PROFILE_STEPS] {
        run += 1;
        k += 1;
    }
    let mut k = 1;
    while k < PROFILE_STEPS && !profile[(start + PROFILE_STEPS - k) % PROFILE_STEPS] {
        run += 1;
        k += 1;
    }
    run as f32 * step
}

/// Every unlit run around the profile, in degrees. On a closed ring of
/// indicators the only unlit stretches are the Octave gap's slits, one per
/// boundary between neighbours — so counting these counts the indicators,
/// and a missing one shows as two slits merged into a wider hole.
fn unlit_runs(profile: &[bool]) -> Vec<f32> {
    let step = 360.0 / PROFILE_STEPS as f32;
    // Start from a lit sample so the walk cannot begin mid-run and count one
    // run as two.
    let from = profile.iter().position(|b| *b).expect("something lit to measure from");
    let mut runs = Vec::new();
    let mut run = 0;
    for k in 0..PROFILE_STEPS {
        if profile[(from + k) % PROFILE_STEPS] {
            if run > 0 {
                runs.push(run as f32 * step);
                run = 0;
            }
        } else {
            run += 1;
        }
    }
    if run > 0 {
        runs.push(run as f32 * step);
    }
    runs
}

/// The invariant the wheel is built around, checked on the picture rather
/// than on the layout that feeds it: every octave of the span gets an
/// indicator and together they close the ring — whatever the counts, the
/// center, the fringe or the node's pitch class. So the only unlit stretches
/// are the Octave gap's slits, one per boundary, and the seam is one of them
/// on every node, wherever that node's turn has carried it.
///
/// Reading it off rendered pixels is the point. The layout's own tests pin
/// the angles down; this one says the shader draws the axis the table
/// describes, in the right direction, anchored where it claims — and in
/// particular that it draws the end indicators out to the seam, which is what
/// closes the ring when the window is not a whole number of octaves.
#[test]
fn every_octave_in_the_range_is_drawn_and_they_close_the_ring() {
    use harmonigraph_scene::octave_layout;

    const SIZE: [u32; 2] = [512, 512];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };

    // The widest wheel, the default, an even count — where the ring reaches an
    // octave further on one side — a center that is neither a C nor near the
    // middle of the keyboard, where the ring names octaves the packing has no
    // room for, and three fringed wheels: the narrowest there is (where the
    // one full-size octave takes a sector widest and the union branch of the
    // wedge test is stressed hardest), a plain register with a pair either
    // side, and a deep fringe filling the budget. Each at a C node (whose
    // octaves land flush on the center) and at three pitch classes that do
    // not, one of them the tritone that turns furthest.
    //
    // An even wheel, a flat fringe, a graded one, and then a fringe thin
    // enough to be eaten by the Octave gap.
    const FRINGES: [(f32, f32); 4] = [(1.0, 0.0), (0.6, 0.0), (0.6, 1.0), (0.15, 0.0)];
    for (count, extras, center) in [
        (11u32, 0u32, 60.0f32),
        (5, 0, 60.0),
        (4, 0, 60.0),
        (5, 0, 103.0),
        (1, 1, 60.0),
        (5, 2, 60.0),
        (3, 4, 60.0),
    ] {
        for (i, &(size, blend)) in FRINGES.iter().enumerate() {
            // Both fringe settings are inert without extras, so the other
            // three would render the same picture at four times the cost.
            if extras == 0 && i > 0 {
                continue;
            }
            for cents in [0.0, 350.0, 600.0, 1150.0] {
                let layout = octave_layout(count, center, extras, size, blend);
                let px = gpu.shot(&octave_wheel_scene(layout, cents));
                let profile = band_profile(&px, SIZE[0]);
                let case =
                    format!("{count}+2x{extras} at {center}, size {size} blend {blend}, {cents}c");

                // One indicator per octave of the wheel, closing the ring:
                // that is one slit per boundary and no other break. A missing
                // indicator merges two slits into one hole, so the count is
                // what says all of them are there — including the ones drawn
                // for octaves no note can reach.
                let want = layout.span as usize;
                let runs = unlit_runs(&profile);
                // Except under a thin fringe, and that is the settings talking
                // rather than a missing indicator: the Octave gap is cut out of every
                // sector from both sides at full width, so an extra thinner
                // than twice that padding has its two slits meet and reads as
                // no indicator at all. At 0.6 of an even slice they still
                // resolve; 0.15 is where they go, and only the extras are ever
                // that thin. `octaves.rs` pins the count exactly, on angles,
                // where no padding is involved.
                if size >= 0.4 {
                    assert_eq!(runs.len(), want, "{case}: unlit runs {runs:?} for {want} sectors");
                } else {
                    let lost = 2 * extras as usize;
                    assert!(
                        runs.len() + lost >= want && runs.len() <= want,
                        "{case}: unlit runs {runs:?} for {want} sectors — at most the \
                         {lost} extras can be lost to the Octave gap"
                    );
                }

                // The seam TURNS with the node: it is the bottom only for the
                // center's own pitch class, and every other class carries it
                // round by however far its octaves sit from the center. Read
                // off the layout rather than at a fixed 270 degrees, which is
                // the whole difference between this wheel and a window.
                let seam = layout.ring(cents).seam.to_degrees().rem_euclid(360.0);
                assert!(gap_at(&profile, seam) > 0.0, "{case}: no seam at {seam:.1} deg");

                // The CENTER pitch is straight up on every node, so a slice
                // covers the top — except on the node exactly a tritone from
                // it, where the center is a boundary and a slit there is the
                // axis being read rather than a hole.
                let tritone = (cents - 600.0).abs() < 1e-3;
                if !tritone {
                    assert!(gap_at(&profile, 90.0) == 0.0, "{case}: nothing covers the top");
                }
            }
        }
    }
}

/// Which PITCH each indicator is drawn at — the whole of what "positioned by
/// absolute pitch" means, and the part a seam test cannot see. One octave
/// sounds; the bright arc has to land where the layout puts that pitch, and
/// a node's pitch class has to move it.
#[test]
fn an_indicator_is_drawn_at_its_own_pitchs_angle() {
    use harmonigraph_scene::octave_layout;

    let Some((device, queue)) = headless_device() else {
        return;
    };
    const SIZE: [u32; 2] = [512, 512];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let vec_size = egui::vec2(SIZE[0] as f32, SIZE[1] as f32);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec_size);
    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };

    let mut pane = 100;
    for (count, extras, center, size, blend) in
        [(5u32, 0u32, 60.0f32, 1.0, 0.0), (8, 1, 66.0, 0.3, 0.0)]
    {
        let layout = octave_layout(count, center, extras, size, blend);
        // A C node and a node a fifth up: same slot, pitches 7 semitones
        // apart, so the bright arc must move by exactly that much of the axis.
        // The octave holding the center pitch, and one further round the
        // wheel, where a wrong anchor or a wrong direction shows.
        //
        // Both held INSIDE the ring rather than at its edges: a thin fringe
        // leaves the extras narrower than the Octave gap's slits, and a centroid
        // needs an arc to measure. That the edges reach the seam at all is
        // `every_octave_in_the_range_is_drawn_and_they_close_the_ring`.
        for (cents, offset) in [(0.0f32, 0i32), (700.0, 0), (0.0, 2), (700.0, 2)] {
            let (first, last) = layout.slots(cents);
            let slot =
                (harmonigraph_scene::MIDDLE_C_SLOT as i32 + offset).clamp(first + 1, last - 1);
            let mut scene = octave_wheel_scene(layout, cents);
            // One octave sounding. The silent slots still carry the ring's
            // ground behind it, which the brightness threshold below sorts
            // out.
            scene.nodes[0].octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
            scene.nodes[0].octaves[slot as usize] = 1.0;

            let cb = LatticeCallback::from_scene(
                &scene,
                LatticeLabels::default(),
                vec_size,
                format,
                pane,
                None,
            );
            pane += 1;
            let mut encoder = device.create_command_encoder(&Default::default());
            let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
            queue.submit(bufs.into_iter().chain([encoder.finish()]));
            let tex =
                render_to_texture(&device, &queue, SIZE, format, wgpu::Color::BLACK, |pass| {
                    cb.paint(
                        egui::PaintCallbackInfo {
                            viewport: rect,
                            clip_rect: rect,
                            pixels_per_point: 1.0,
                            screen_size_px: SIZE,
                        },
                        pass,
                        &resources,
                    );
                });
            let px = readback(&device, &queue, &tex, SIZE);

            // Where the BRIGHT pixels are, as a mean direction. The lit
            // indicator runs several times the ghosts' level, so half the
            // maximum separates them cleanly whatever the node color is.
            let bright = |i: usize| px[i] as u32 + px[i + 1] as u32 + px[i + 2] as u32;
            let peak = (0..px.len() / 4).map(|k| bright(k * 4)).max().unwrap_or(0);
            assert!(peak > 60, "{count}+2x{extras} at {center}: nothing bright enough");
            let (mut vx, mut vy) = (0f64, 0f64);
            let c = SIZE[0] as f64 / 2.0;
            for y in 0..SIZE[1] {
                for x in 0..SIZE[0] {
                    let i = ((y * SIZE[0] + x) * 4) as usize;
                    if bright(i) > peak / 2 {
                        // Screen y grows downward; flip it for an ordinary angle.
                        vx += x as f64 - c;
                        vy += c - y as f64;
                    }
                }
            }
            let drawn = vy.atan2(vx).to_degrees() as f32;
            // The indicator's own middle, from the layout: the pitch halfway
            // between its two edges in ANGLE, which a fringe can shift off the
            // pitch itself.
            let (e0, e1) = layout.sector(slot, cents);
            let expected = (0.5 * (e0 + e1)).to_degrees().rem_euclid(360.0);
            let off = (drawn.rem_euclid(360.0) - expected).rem_euclid(360.0);
            let off = off.min(360.0 - off);
            assert!(
                off < 6.0,
                "{count}+2x{extras} at {center}, {cents}c, slot {slot}: indicator drawn \
                 at {drawn:.1} deg, the axis puts its pitch at {expected:.1}"
            );
        }
    }
}

/// How a release ENDS: the fading indicator arrives at the grey its silent
/// neighbours are drawn in, and arrives there continuously.
///
/// It only shows where the node's PRESENCE outlives this slot's level, which
/// is another instance of the pitch class still held. A lone note drives the
/// backdrop and the lit glyph off one envelope, so both land at nothing
/// together and any discontinuity between them has no coverage left to show
/// in; here the held octave pins the backdrop at full strength while the
/// released one runs out against it.
///
/// A slot painted in PLACE of its ghost — opacity by a max(), color by a
/// `level > 0` switch — is caught below, and knowing WHICH check catches it is
/// worth stating. The ghost is the rings' own grey (`Scene::lattice_ground`),
/// and here it is darker than anything this fixture paints over it: every
/// `pitch_lut` entry adds up to 1.4 across its three channels, against the
/// fresh Ground's grey at 0.57. So the switch's last frame is a step DOWN in
/// light, the never-brightens loop passes the whole way, and the TAIL-SPREAD
/// check is the one that fires — the slice holds its pitch to the last lit
/// frame and then makes the entire journey to the ground in one. The loop
/// takes the fault first only where the ground is the BRIGHTER of the two,
/// which is a Ground bar away rather than a shader change, so the spread check
/// is the one to read this test by. The last check is neither: at level 0 both
/// shaders run the same line, so it can only say the finished ring is one
/// backdrop, not how it got there.
#[test]
fn a_released_octave_lands_on_its_ghost_without_a_step() {
    use harmonigraph_scene::octave_layout;

    const SIZE: [u32; 2] = [384, 384];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    // An even five-octave wheel on a C node: a slice is 72 degrees, which is
    // room to sample well inside one and well inside its neighbour.
    let layout = octave_layout(5, 60.0, 0, 1.0, 0.0);
    let held = harmonigraph_scene::MIDDLE_C_SLOT;
    let (releasing, silent) = (held + 1, held + 2);
    // All three inside the ring this wheel draws. `sector` CLAMPS a slot
    // outside it rather than refusing, so a wheel that stopped reaching them
    // would leave the neighbour reading below comparing one slice against
    // itself — passing for a reason that has nothing to do with the fade.
    let (low, high) = layout.slots(0.0);
    for slot in [held, releasing, silent] {
        assert!((low..=high).contains(&(slot as i32)), "slot {slot} is outside {low}..={high}");
    }
    let scene = |level: f32| {
        let mut scene = octave_wheel_scene(layout, 0.0);
        let node = &mut scene.nodes[0];
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        node.octaves[held] = 1.0;
        node.octaves[releasing] = level;
        // The instance still down. This is what holds the whole backdrop up
        // while the other one fades, and the reason the handoff is legible.
        node.activation = 1.0;
        scene
    };

    let (mid, wedge) = wedge_of(layout, releasing, 0.0);
    // Down the envelope past the ghost's own level, to the smallest the 8-bit
    // packing carries, and then off it. The first of these is also the shot
    // the radii are calibrated from, taken once and read twice so the two can
    // never drift onto different pictures.
    const TAIL: [f32; 9] = [1.0, 0.5, 0.25, 0.16, 0.12, 0.08, 0.04, 0.02, 1.0 / 255.0];
    let full = gpu.shot(&scene(TAIL[0]));
    let probe = BandProbe::new(&full, SIZE, mid);

    let mut steps: Vec<[f32; 3]> = vec![probe.mean(&full, mid, wedge)];
    steps.extend(TAIL[1..].iter().map(|&level| probe.mean(&gpu.shot(&scene(level)), mid, wedge)));
    let ended = gpu.shot(&scene(0.0));
    steps.push(probe.mean(&ended, mid, wedge));

    let light = |c: &[f32; 3]| c[0] + c[1] + c[2];
    let apart = |a: &[f32; 3], b: &[f32; 3]| {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    };
    // The envelope only ever takes light out of the slice.
    for pair in steps.windows(2) {
        assert!(
            light(&pair[1]) <= light(&pair[0]) + 0.5,
            "the fade brightens at {:?} then {:?}",
            pair[0],
            pair[1]
        );
    }
    // And the last stretch of it — the bottom sixth of the envelope, where a
    // slice is nearly the ground already — is SPREAD across the frames rather
    // than spent in one. Painted in place of the ground instead of mixed
    // toward it, the slice sits still for that whole stretch and then makes
    // the entire journey in one frame.
    //
    // A share of the travel rather than a run of strict decreases: a ramp
    // this shallow moves the last few frames by less than an 8-bit channel,
    // and the pair either side of zero reads identically here BECAUSE the
    // handoff is smooth. Where the cut falls is not the claim — a different
    // one only widens or narrows the stretch measured, so this reads the
    // sweep rather than asserting a level.
    let tail = &steps[TAIL.iter().position(|&level| level <= 0.16).expect("a tail to measure")..];
    let travel = apart(&tail[0], &tail[tail.len() - 1]);
    assert!(travel > 10.0, "the tail hardly moves at all ({travel:.1}), so its shape says little");
    for pair in tail.windows(2) {
        let step = apart(&pair[0], &pair[1]);
        assert!(
            step < 0.4 * travel,
            "the indicator spends {step:.1} of its {travel:.1} tail in one step, at {:?}",
            pair[1]
        );
    }
    // Landing on the ground the silent slices are drawn in — the same grey at
    // the same coverage, so the finished ring is one backdrop rather than a
    // backdrop with one slice a shade off it.
    let (quiet, quiet_wedge) = wedge_of(layout, silent, 0.0);
    let neighbour = probe.mean(&ended, quiet, quiet_wedge);
    assert!(
        apart(&steps[steps.len() - 1], &neighbour) < 3.0,
        "a spent indicator reads {:?} against its neighbours' {neighbour:?}",
        steps[steps.len() - 1]
    );
}

/// The OTHER release: a node going out together with its octave, which is a
/// lone note let go, or the last instance of one. Here `level` and `presence`
/// are the same envelope and there is no backdrop to hand off to — it is
/// leaving too — so the indicator keeps its own pitch the whole way down and
/// its opacity runs to nothing in a STRAIGHT line.
///
/// That is the half a ghost scaled by `1 - level` gets wrong. It is the same
/// arithmetic wherever presence is 1, so the handoff above cannot see it, but
/// with the two on one envelope it counts the note's presence twice: the
/// opacity bulges to `1.16e - 0.16e²`, four points over the line at the middle
/// of the fade, and the slice picks up a whitening from a backdrop that is not
/// there. Taking the ghost as what is LEFT of the presence after this slot's
/// own level is what makes both releases straight.
#[test]
fn a_lone_notes_octave_fades_in_a_straight_line() {
    use harmonigraph_scene::octave_layout;

    const SIZE: [u32; 2] = [384, 384];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let layout = octave_layout(5, 60.0, 0, 1.0, 0.0);
    let slot = harmonigraph_scene::MIDDLE_C_SLOT;
    let scene = |envelope: f32| {
        let mut scene = octave_wheel_scene(layout, 0.0);
        let node = &mut scene.nodes[0];
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        node.octaves[slot] = envelope;
        // ONE envelope for both: nothing else on this node is sounding, so
        // its presence is this octave's own.
        node.activation = envelope;
        scene
    };

    let (mid, wedge) = wedge_of(layout, slot, 0.0);
    let full = gpu.shot(&scene(1.0));
    let probe = BandProbe::new(&full, SIZE, mid);
    let lit = probe.mean(&full, mid, wedge);

    // Proportional, channel by channel: the wedge is nothing but this glyph
    // (no core and no rings, and an idle node paints nothing at all), so a
    // straight-line fade is the reading at `e` being `e` of the reading at
    // full. The tolerance is the 8-bit packing of the level plus the target's
    // own rounding, well under the 5-to-9 the bulge would add here.
    for envelope in [0.75f32, 0.5, 0.25] {
        let got = probe.mean(&gpu.shot(&scene(envelope)), mid, wedge);
        for j in 0..3 {
            let want = envelope * lit[j];
            assert!(
                (got[j] - want).abs() < 2.5,
                "at {envelope} of the envelope the slice reads {got:?}, not {want:.1} in \
                 channel {j} — {lit:?} at full"
            );
        }
    }
    // And it ends at nothing rather than on a ghost: with the node gone there
    // is no backdrop left for the indicator to sit in.
    let spent = probe.mean(&gpu.shot(&scene(0.0)), mid, wedge);
    assert!(spent.iter().sum::<f32>() < 3.0, "a spent lone note leaves {spent:?} behind");
}

/// The ground reaches the shader as a UNIFORM, and the picture has to track
/// it: a silent slice wears the grey `Scene::lattice_ground` carries, whatever
/// that is, rather than one grey baked into the shader.
///
/// Every other fixture here draws at the fresh Ground alone, and the grey that
/// names is `vec3(0.189)` — near enough a plausible literal that a shader
/// ignoring `u.lattice_ground` entirely, or reading the ground out of the
/// wrong vec4 of the uniform block, would render all of them pixel for pixel.
/// So this one draws one node four times across the bar, the fresh 20 first
/// and then a near-black, a mid grey and a near-white.
///
/// Full presence against a slot at level 0 is where the arithmetic leaves
/// nothing to interpret: a silent slice's opacity IS the node's presence, so
/// at 1.0 the wedge is the ground colour undiluted, and the byte is that
/// colour at 8 bits. The LIT slot beside it is read from the same shot and has
/// to stay put — at full level the ghost is nothing, so a sounding pitch owes
/// the ground no part of its colour, and a ground that moved it would be the
/// mix leaking into the one place it must not reach.
///
/// A channel and a half, tighter than the 2.5 the fade probes above allow, and
/// honestly so: those read points ON an envelope, where the level's own 8-bit
/// packing is inside the measurement, and nothing here fades. One flat colour
/// into an 8-bit target rounds by half a channel and by nothing else, and the
/// closest pair of grounds measured lands 29 channels apart — twenty times the
/// tolerance — so nothing here passes by being loose.
#[test]
fn a_silent_slice_wears_the_ground_the_scene_names() {
    use harmonigraph_scene::{grey_of_lightness, octave_layout};

    const SIZE: [u32; 2] = [384, 384];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    // The even five-octave wheel the release tests use: a 72-degree slice is
    // room to sample well inside one and well inside its neighbour.
    let layout = octave_layout(5, 60.0, 0, 1.0, 0.0);
    let lit = harmonigraph_scene::MIDDLE_C_SLOT;
    let quiet = lit + 1;
    // Both inside the ring this wheel draws: `sector` CLAMPS a slot outside it
    // rather than refusing, which would leave the two readings below taken on
    // one wedge and agreeing for a reason that has nothing to do with the
    // ground.
    let (low, high) = layout.slots(0.0);
    for slot in [lit, quiet] {
        assert!((low..=high).contains(&(slot as i32)), "slot {slot} is outside {low}..={high}");
    }
    let scene_at = |ground: f32| {
        let mut scene = octave_wheel_scene(layout, 0.0);
        scene.lattice_ground = grey_of_lightness(ground);
        let node = &mut scene.nodes[0];
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        node.octaves[lit] = 1.0;
        // The note fully down, so every silent slice on the ring is opaque and
        // the wedge below reads the ground rather than a share of it.
        node.activation = 1.0;
        scene
    };

    let (lit_mid, lit_wedge) = wedge_of(layout, lit, 0.0);
    let (quiet_mid, quiet_wedge) = wedge_of(layout, quiet, 0.0);
    let mut pitch: Option<[f32; 3]> = None;
    for ground in [20.0f32, 6.0, 45.0, 80.0] {
        let px = gpu.shot(&scene_at(ground));
        // Calibrated along the SOUNDING slice, which is the one ray that is
        // bright at every ground: a band drawn in a near-black ground is the
        // dark end of the bar, and the radii are the scene's either way.
        let probe = BandProbe::new(&px, SIZE, lit_mid);
        let got = probe.mean(&px, quiet_mid, quiet_wedge);
        let want = grey_of_lightness(ground).truncate() * 255.0;
        for j in 0..3 {
            assert!(
                (got[j] - want[j]).abs() < 1.5,
                "at Ground {ground} the silent slice reads {got:?}, not {want:?}"
            );
        }
        let sounding = probe.mean(&px, lit_mid, lit_wedge);
        match pitch {
            None => pitch = Some(sounding),
            Some(first) => {
                for j in 0..3 {
                    assert!(
                        (sounding[j] - first[j]).abs() < 1.5,
                        "at Ground {ground} the lit slice reads {sounding:?}, and at the \
                         first ground it read {first:?}"
                    );
                }
            }
        }
    }
}

/// The seams between a chord's colors run at ONE width from the edge of a
/// node's light to its middle. They are laid down as lobes of fixed ANGULAR
/// width, so the arc each spans shrinks with the radius and they would
/// otherwise converge to a cusp at the node's centre — sharpest exactly where
/// the node has the fewest pixels to say it with.
///
/// Both halves of the bargain, because either alone has a trivial cheat: the
/// centre has to lose its seam, AND the outside has to keep its colors, which
/// is what stops the cure from being "average the whole halo".
///
/// The node's light is the only place this can be read. It is the one thing a
/// node draws whose colour is laid in ANGLE at every radius at once — every
/// ring paints its own annulus and nothing inside it — so the cusp is the
/// glow's own to have, and `glow_layer`'s ease toward the strip's mean is the
/// cure being measured.
///
/// Measured as how far the colors around a ring point APART as directions, not
/// as how much they differ: the light dims inward under the Centre dip and
/// outward under its falloff, and any measure of magnitude would read that
/// dimming as a blur and pass on it.
#[test]
fn the_lights_colour_seams_run_at_one_width_from_its_edge_to_the_centre() {
    const SIZE: [u32; 2] = [512, 512];
    // Inside the node's own middle, where the light runs in to the centre with
    // nothing standing it off, and out past every ring the node draws, where
    // the light is all there is. Both are pure light: the node's ink is an
    // annulus between them (the octave band, 80..120 px at this node size), and
    // a reading taken on it would be the band's colour rather than the halo's.
    const INNER: f32 = 20.0;
    const OUTER: f32 = 170.0;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };

    let mut scene = single_marked_node(0, 0);
    // Every octave the wheel draws for this pitch class. A single sounding
    // voice takes the node's own color everywhere (octave_glow_color's solo
    // fallback), which leaves no seam to measure at all.
    let layout = scene.octave_layout;
    let node = &mut scene.nodes[0];
    let (low, high) = layout.slots(node.cents);
    node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
    for slot in low.max(0)..=high.min(harmonigraph_scene::OCTAVE_SLOTS as i32 - 1) {
        node.octaves[slot as usize] = 1.0;
    }
    // The octave band is the ink the light's colour is read off, and it is the
    // only layer on: nothing is drawn inside it, so every pixel sampled below
    // is light and nothing else.
    scene.mark_thickness = 0.0;
    scene.spectral = harmonigraph_scene::SpectralPaint::silent();
    scene.node_radius = 1.6;
    scene.glow_reach = 0.8;
    scene.glow_strength = 1.5;
    // Each octave's hue kept as its own arc rather than averaged round the
    // halo, which is the state a seam exists in at all.
    scene.glow_blend = 0.0;
    let px = shooter.shot(&scene);

    // The node is alone at the world origin and the camera looks at it, so the
    // frame's centre is its centre.
    let c = (SIZE[0] / 2) as i32;
    let rgb = |x: i32, y: i32| -> glam::Vec3 {
        let i = ((y as u32 * SIZE[0] + x as u32) * 4) as usize;
        glam::Vec3::new(px[i] as f32, px[i + 1] as f32, px[i + 2] as f32) / 255.0
    };
    // How far apart, in degrees, the most divergent pair of colors around a
    // ring of radius `r` point. Zero is one flat color all the way round.
    let spread = |r: f32| -> f32 {
        let dirs: Vec<glam::Vec3> = (0..64)
            .map(|i| {
                let a = std::f32::consts::TAU * i as f32 / 64.0;
                // Screen y grows downward; the sample angle is negated.
                rgb(c + (r * a.cos()).round() as i32, c - (r * a.sin()).round() as i32)
            })
            .filter(|v| v.length() > 0.02)
            .map(|v| v.normalize())
            .collect();
        let lit = dirs.len();
        assert!(lit > 56, "the ring at r={r:.0} is not lit: {lit} lit samples of 64");
        let mut worst = 0.0f32;
        for (i, a) in dirs.iter().enumerate() {
            for b in &dirs[i + 1..] {
                worst = worst.max(a.dot(*b).clamp(-1.0, 1.0).acos().to_degrees());
            }
        }
        worst
    };

    let (at_centre, at_edge) = (spread(INNER), spread(OUTER));
    eprintln!("seams: {at_centre:.0} deg at {INNER} px, {at_edge:.0} at {OUTER}");
    // No cusp: the middle is a blend rather than the point where every seam
    // meets. This is what fails if the mix toward the strip's mean goes away
    // and the light is laid at one fixed concentration — the centre then reads
    // as separated as the edge.
    assert!(
        at_centre < at_edge * 0.5,
        "the seams still converge — {at_centre:.0} deg across the centre against \
         {at_edge:.0} further out",
    );
    // And what stops the cure being "average the node": the seams are never
    // held wider than the arc they already span out where the ink is, so the
    // node still shows its notes as distinct colors.
    // The arc is the strip's own — the light is a BLEND of the chord's hues by
    // design, and out where the mix reaches the strip in full it spans the arc
    // GLOW_LOBE_KAPPA gives it. What this rules out is that arc collapsing to
    // the flat mean everywhere, which is what "average the node" looks like.
    assert!(
        at_edge > 15.0,
        "the colors washed out instead of their seams widening — only {at_edge:.0} deg \
         across the outer ring",
    );
}
