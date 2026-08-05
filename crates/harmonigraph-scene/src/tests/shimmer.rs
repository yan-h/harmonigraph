//! The shimmer's clock: how a song position becomes the one number the
//! shader gets to see (see [`Scene::shimmer_slide`]).
//!
//! Both tests here are about a transport an hour or more into a piece, which
//! is the only place either defect shows and is an ordinary place for this
//! plugin to be: a render of a long take starts at 0 and ends wherever the
//! music does.

use crate::*;
use harmonigraph_core::Tuning;
use super::harness::*;

/// The sheet's position at `now`, taken the way a frame takes it — through
/// `derive_scene`, so the clock's route into the scene is under test as much
/// as the arithmetic at the end of it.
fn slide_at(speed: f32, width: f32, now: f64) -> f32 {
    let view = ViewConfig { shimmer_speed: speed, shimmer_width: width, ..ViewConfig::default() };
    scene_of(&sounding(), &Tuning::default(), &view, &FrameParams::default(), now).shimmer_slide()
}

/// The sheet travels at one steady rate, with no instant anywhere in a long
/// session where it jumps instead.
///
/// A clock the shader has to wrap for itself is what puts one there: wrapped
/// at an hour, the sheet crosses the wrap mid-band unless the speed and the
/// width happen to divide 3600 between them — 1.6 and 5.0 do (1152 periods
/// exactly), 1.6 and 4.3 do not (1339.53), and 3600 being highly composite
/// that is a lot of the round settings and none of the rest. A seam at some
/// settings and not others reads as a bar being unstable rather than as a
/// known cost, which is worse than a seam at all of them.
#[test]
fn the_shimmer_crosses_the_hour_without_a_seam() {
    // A frame at 60fps: the step a jump would have to hide inside.
    const STEP: f64 = 1.0 / 60.0;
    for (speed, width) in [(1.6, 5.0), (1.6, 4.3), (1.6, 0.35), (6.0, 0.05), (0.7, 12.0)] {
        let cycle = 2.0 * width as f64;
        // Either side of the hour, and far enough to catch a wrap that lands
        // off the exact second.
        let mut now = 3600.0 - 20.0 * STEP;
        while now < 3600.0 + 20.0 * STEP {
            let travelled = f64::from(slide_at(speed, width, now + STEP))
                - f64::from(slide_at(speed, width, now));
            // The sheet's own cycle is the unit here: it is periodic, so an
            // advance and that advance plus a cycle are the same picture, and
            // only the distance ROUND the cycle is a jump.
            let want = STEP * speed as f64;
            let off = (travelled - want).rem_euclid(cycle);
            let off = off.min(cycle - off);
            // Two ULP-ish of the f32 the slide is carried in at the widest
            // setting here, and well under what either defect costs: the
            // hourly seam is worth a good fraction of a period, and the f32
            // phase misses by 1.9e-4 even at a setting whose period divides
            // the wrap exactly.
            assert!(
                off < 1e-4,
                "at speed {speed} width {width}, the frame at {now:.4}s moved the sheet \
                 {travelled:.6} world units where the clock asks for {want:.6} — a jump of \
                 {off:.6}, which is {:.1}% of a period",
                100.0 * off / width as f64,
            );
            now += STEP;
        }
    }
}

/// An hour in, the sheet still has somewhere to be between one band and the
/// next.
///
/// The thing being kept out is a phase carried as an absolute distance in
/// f32: its ULP grows with the clock while the period it is measured against
/// is a bar setting that reaches 0.05, so at the top of one bar and the
/// bottom of the other a sheet late in an hour has about 26 distinct
/// positions per band and stair-steps through them. Reduced onto its own
/// cycle first, the phase is a small number whatever the clock reads.
#[test]
fn the_shimmer_still_has_a_phase_to_land_on_late_in_a_song() {
    // The corner the two bars make: fastest travel against the tightest
    // period, which is where the steps are coarsest.
    let (speed, width) = (6.0f32, 0.05f32);
    // Just SHORT of the hour, which is where an absolute f32 phase is at its
    // coarsest — a clock wrapped at 3600 is exact again the moment it wraps,
    // so a fixture sitting on the wrap measures the one instant of the hour
    // that has nothing wrong with it.
    const NOW: f64 = 3599.0;
    // One band's worth of clock, walked in 128 steps. Anything that quantizes
    // the phase collapses these onto each other.
    const STEPS: usize = 128;
    let band = (width / speed) as f64;
    let mut seen: Vec<f32> = (0..STEPS)
        .map(|k| slide_at(speed, width, NOW + band * k as f64 / STEPS as f64))
        .collect();
    seen.sort_by(f32::total_cmp);
    seen.dedup();
    assert!(
        seen.len() >= STEPS - 1,
        "only {} of {STEPS} clocks an hour in put the sheet somewhere different: \
         the phase is quantized into steps a band can be seen crossing (an f32 \
         phase in absolute world units leaves about 26 of them)",
        seen.len(),
    );
}
