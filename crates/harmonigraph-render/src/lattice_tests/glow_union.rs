//! How two lights add where they meet: the p-norm union `fs_glow_gather` folds
//! the halos with.
//!
//! Both claims are read on a BARE part of the frame — outside every node's ink,
//! over a black ground, with the Shadow off — so a pixel is the light and
//! nothing else, and each fixture's first assertion is that the reading really
//! is zero without it.
//!
//! Both are read off SUMS over many pixels rather than one, because the effect
//! they measure is a few percent and a single 8-bit pixel is worth half a level
//! either way.

use super::fixtures::*;
use crate::*;

const SIZE: [u32; 2] = [256, 256];

/// The exponent the halos are unioned at — `GLOW_UNION` in lattice.wgsl,
/// restated rather than read out of the source: a test that took the shader's
/// own number could not fail when the shader's number moved.
const P: f64 = 8.0;

/// Where the pair below is read: the middle of the pane, on the centre of one
/// pixel, which is where each fixture pans the world origin to.
const CENTRE: glam::Vec2 = glam::Vec2::new(SIZE[0] as f32 * 0.5 + 0.5, SIZE[1] as f32 * 0.5 + 0.5);

/// Put the world origin on the centre of the middle pixel.
///
/// Every reading here is taken about that point, and the pane's width is even,
/// so its geometric middle falls BETWEEN two fragments — half a pixel of
/// asymmetry, which at an exponent of 8 is worth more than the effect being
/// measured.
fn on_the_middle_pixel(scene: &mut Scene) {
    for _ in 0..12 {
        let error = CENTRE - on_screen(scene, SIZE, glam::Vec3::ZERO);
        scene.camera.pan(error);
    }
    let landed = on_screen(scene, SIZE, glam::Vec3::ZERO);
    assert!(landed.distance(CENTRE) < 0.001, "the origin landed at {landed:?}, not {CENTRE:?}");
}

/// One node uv in screen pixels, which is what every radius below is stated in.
fn pixels_per_uv(scene: &Scene) -> f32 {
    let world = scene.node_radius * 1.8;
    on_screen(scene, SIZE, glam::Vec3::new(world, 0.0, 0.0)).x
        - on_screen(scene, SIZE, glam::Vec3::ZERO).x
}

/// A lit lattice with the two things that would answer for light on a bare
/// pixel taken away: every marker, and the Shadow.
fn bare_lattice(strength: f32) -> Scene {
    let mut scene = single_marked_node(0, 0);
    scene.glow_reach = 0.8;
    scene.glow_strength = strength;
    scene.shadow = one_shadow(0.0, 0.0, harmonigraph_scene::ShadowKernel::Gaussian);
    scene
}

/// Where two equal halos MEET, the light is `2^(1/p)` times what one of them
/// puts there — 9% at the exponent this folds at, and not the 80% the screen
/// blend it replaces would have laid down.
///
/// This is #680's ask in one measurement: a second note beside the first does
/// not make the region between them brighter in proportion, it makes the lit
/// region larger.
///
/// Two copies of one node on the screen's x, two node uv apart, with their
/// MIDPOINT on the centre of a pixel. The reading is a run of that pixel's own
/// column, which is the pair's perpendicular bisector: every pixel of it is
/// exactly as far from one node as from the other, and the claim is about
/// EQUAL coverages — the union of two unequal ones is not `2^(1/p)` times
/// either. Half a pixel of asymmetry is not small here, which is what the pan
/// is for.
///
/// Held against the MEAN of the two nodes read alone rather than against
/// either. The two are equally far from the column but see it from opposite
/// directions, so each lays down its own strip's colour at a different angle
/// and the two readings differ in hue. What the union does with that is take
/// the mean colour, weighted by the same terms as the norm — so at equal
/// coverage the summed brightness is exactly the mean of the two, scaled by the
/// norm's own gain, whatever either colour is.
///
/// The tolerance is 1.5% against a 9% effect — six times apart, so this fails
/// on any other fold at this geometry: the screen blend it replaces reads 73%
/// over one node here (measured), a plain max would read 0%. The fixture lands
/// within 0.05%, so the slack is for a driver rather than for the claim.
#[test]
fn two_halos_meeting_read_the_norm_of_one_rather_than_their_sum() {
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // Two node uv apart, so the bisector stands one uv from each: outside the
    // rings a node draws (0.795 uv) and inside the span its light reaches
    // (0.795 + the Reach), which is what puts a bare pixel in both halos.
    let half = single_marked_node(0, 0).node_radius * 1.8;
    let at = |lit: [f32; 2]| -> Scene {
        let mut scene = bare_lattice(1.5);
        let node = scene.nodes[0];
        scene.nodes = [-half, half]
            .iter()
            .enumerate()
            .map(|(i, x)| {
                let mut node = node;
                node.world_pos = glam::Vec3::new(*x, 0.0, 0.0);
                node.lattice_pos = harmonigraph_core::LatticePos::new(i as i32, 0, 0);
                // The node is SHIPPED either way and draws its own ink either
                // way; what the fixture turns off is its light alone, so the
                // three shots differ in nothing else — the same two instances,
                // the same two rows of the ink strip.
                node.glow.level = lit[i];
                node
            })
            .collect();
        rows_per_node(&mut scene);
        on_the_middle_pixel(&mut scene);
        scene
    };
    // The bisector, out to where both halos are still well inside their span.
    let bisector = |shot: &[u8]| -> i64 {
        let column = SIZE[0] as usize / 2;
        (SIZE[1] as usize / 2 - 40..=SIZE[1] as usize / 2 + 40)
            .map(|row| {
                let px = (row * SIZE[0] as usize + column) * 4;
                brightness(&shot[px..px + 4])
            })
            .sum()
    };

    let dark = bisector(&shooter.shot(&at([0.0, 0.0])));
    assert_eq!(dark, 0, "the read column is not bare: {dark} of something that is not the light");
    let left = bisector(&shooter.shot(&at([1.0, 0.0])));
    let right = bisector(&shooter.shot(&at([0.0, 1.0])));
    let both = bisector(&shooter.shot(&at([1.0, 1.0])));
    assert!(
        left > 0 && right > 0,
        "a node's halo does not reach the bisector at all ({left} and {right}); the fixture \
         measures one light rather than two meeting",
    );

    let gain = both as f64 / ((left + right) as f64 / 2.0);
    let want = 2f64.powf(1.0 / P);
    assert!(
        (gain - want).abs() < 0.015,
        "two halos meeting read {gain:.4} times one, against the norm's {want:.4} \
         (one node {left} and {right}, both {both})",
    );
}

/// A CLUSTER does not brighten: `n` nodes over one another read at most
/// `n^(1/p)` times one of them at the same pixel, and never less than one of
/// them.
///
/// The other half of #680's ask, and the one the screen blend failed worst: `n`
/// coverages of `a` under screen read `1 - (1-a)^n`, which is nearly `n` times
/// one while `a` is small and climbs toward white as notes are added. The
/// fixture's own coverage is measured rather than assumed, so the screen figure
/// below is this fixture's and not a general one.
///
/// Four copies of one node at ONE position, so each lays down the same coverage
/// and the same colour at every pixel and the reading is the operator alone
/// with no geometry in it. Read over the annulus from one to one and a half
/// node uv: outside the rings and inside the span, all the way round.
///
/// How the coverage is bounded: the Strength scales it linearly and clamps at
/// full, so a shot at four times the Strength reads `min(4a, 1)` over `a` —
/// which is at most `1/a` however much clamped, and so bounds `a` by the
/// reciprocal of whatever it reads. Taken at ONE pixel, on the annulus's inner
/// edge where the falloff leaves the coverage largest, so the bound holds over
/// the whole ring.
#[test]
fn a_cluster_of_nodes_spreads_its_light_without_brightening_it() {
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    const N: usize = 4;
    // Low enough that four times it is still inside the coverage's own clamp,
    // which is what the bound below needs.
    const STRENGTH: f32 = 0.5;
    let at = |lit: usize, strength: f32| -> Scene {
        let mut scene = bare_lattice(strength);
        let node = scene.nodes[0];
        scene.nodes = (0..N)
            .map(|i| {
                let mut node = node;
                // One position, distinct lattice cells: the light is a function
                // of where a node IS, and these are all in one place.
                node.lattice_pos = harmonigraph_core::LatticePos::new(i as i32, 0, 0);
                node.glow.level = f32::from(i < lit);
                node
            })
            .collect();
        rows_per_node(&mut scene);
        on_the_middle_pixel(&mut scene);
        scene
    };
    let uv = pixels_per_uv(&at(1, STRENGTH));
    let ring = |shot: &[u8]| -> i64 {
        let mut sum = 0;
        for row in 0..SIZE[1] as usize {
            for column in 0..SIZE[0] as usize {
                let at = glam::vec2(column as f32 + 0.5, row as f32 + 0.5).distance(CENTRE);
                if at >= uv && at < uv * 1.5 {
                    let px = (row * SIZE[0] as usize + column) * 4;
                    sum += brightness(&shot[px..px + 4]);
                }
            }
        }
        sum
    };
    // The annulus's inner edge, where the falloff leaves the coverage largest.
    let inner = |shot: &[u8]| -> i64 {
        let column = (CENTRE.x + uv).floor() as usize;
        let px = ((SIZE[1] as usize / 2) * SIZE[0] as usize + column) * 4;
        brightness(&shot[px..px + 4])
    };

    let dark = ring(&shooter.shot(&at(0, STRENGTH)));
    assert_eq!(dark, 0, "the read ring is not bare: {dark} of something that is not the light");
    let one_shot = shooter.shot(&at(1, STRENGTH));
    let (one, all) = (ring(&one_shot), ring(&shooter.shot(&at(N, STRENGTH))));
    assert!(one > 0, "one node lights nothing in the ring; there is no cluster to measure");

    // One node's coverage, bounded from above by the headroom a fourfold
    // Strength has. The reading is `min(4a, 1)` over `a`, so a reading of at
    // least HEADROOM gives `a * HEADROOM <= min(4a, 1) <= 1` however much
    // clamping there was — the floor rather than the ratio itself, so a level of
    // 8-bit rounding cannot make the bound tighter than it is. The fixture
    // measures 4.02 against this 3.5.
    const HEADROOM: f64 = 3.5;
    let loud = shooter.shot(&at(1, STRENGTH * 4.0));
    let scaled = inner(&loud) as f64 / inner(&one_shot) as f64;
    assert!(
        scaled >= HEADROOM,
        "four times the Strength read {scaled:.2} times as much, so the fixture's coverage is \
         near full and says nothing about how a cluster stacks",
    );
    // What the screen blend this replaces would have read for the same `n` at
    // the same coverage: `1 - (1-a)^n` over `a`, which falls as the coverage
    // rises, so the bound above gives its smallest possible value: 2.59 here
    // against the norm's 1.19. Measured on the fold this replaces, the same
    // fixture reads 3.60, so the assertion below is the one that fails on it.
    let coverage = 1.0 / HEADROOM;
    let screen = (1.0 - (1.0 - coverage).powi(N as i32)) / coverage;

    let gain = all as f64 / one as f64;
    let want = (N as f64).powf(1.0 / P);
    assert!(
        gain <= want * 1.01,
        "{N} nodes at one place read {gain:.4} times one, past the norm's {want:.4}",
    );
    assert!(gain >= 1.0, "{N} nodes read {gain:.4} times one, less than one of them alone");
    assert!(
        gain < screen,
        "{N} nodes read {gain:.4} times one, which the screen blend would have given at a \
         coverage of {coverage:.3} ({screen:.4})",
    );
}
