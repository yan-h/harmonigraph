//! How two lights add where they meet: the p-norm union `fs_glow_gather` folds
//! the halos with.
//!
//! What a note gives off on its own, what two of them come to where they meet,
//! what a cluster of them comes to, and what two of them of different COLOUR
//! come to per channel — the four readings the Union bar is answerable for. The
//! first is the one it must not move at all; the last is the same norm as the
//! others, run per channel on the nodes' premultiplied light, so the exponent
//! reaches the colour as much as it reaches the brightness.
//!
//! The three readings that need more than one node are taken on a BARE part of
//! the frame — outside every node's ink, over a black ground, with the Shadow
//! off — so a pixel is the light and nothing else, and each fixture's first
//! assertion is that the reading really is zero without it. The two RATIOS are
//! read off SUMS over many pixels rather than one, because the effect they
//! measure is a few percent and a single 8-bit pixel is worth half a level
//! either way; the colour is read per channel off each pixel of a run and
//! averaged, which is what keeps that rounding from biasing a norm of two
//! readings taken separately. The lone note is read the other way about, over
//! every byte of the frame, for the reason its own doc gives.

use super::fixtures::*;
use crate::*;

const SIZE: [u32; 2] = [256, 256];

/// The exponent the halos are unioned at, at the value a fresh view opens on —
/// restated rather than read out of `ViewConfig::default`, since a test that
/// took the shipped number could not fail when the shipped number moved.
const P: f64 = 8.0;

/// The Union bar's three readings: its two ends and the fresh middle.
///
/// Both ends are the bar's own ([`harmonigraph_scene::GLOW_UNION_MIN`] and
/// `GLOW_UNION_MAX`), restated here for the same reason `P` is. The gains they
/// stand for are 41%, 9% and 2% over one node.
const SWEEP: [f64; 3] = [2.0, P, 32.0];

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

/// A note ON ITS OWN draws the same frame at every position of the Union bar,
/// to the byte.
///
/// #680's first promise and the half the bar could quietly break. With one
/// contributor the union's sum `s` is exactly 1, and `pow(1, 1/p)` is 1 at
/// every exponent, so the light is that node's own coverage and its own colour
/// whatever the bar says — but only while the power and the root read ONE
/// number. A fold raised at the uniform and rooted at a leftover constant, a
/// clamp spent on one side of the pair, or an exponent reaching into
/// `glow_layer`'s coverage would each show up here and nowhere else in this
/// file, since every other reading is a RATIO of frames drawn at one exponent
/// and a common factor cancels out of all of them.
///
/// Read as byte-identity over the WHOLE frame rather than as a number on a
/// line: with one node there is no bisector to read, and "nothing moved" is
/// worth as many pixels as it is asked over.
#[test]
fn a_lone_halo_is_the_same_frame_at_every_bar_position() {
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |p: f64| -> Scene {
        let mut scene = bare_lattice(1.5);
        scene.glow_union = p as f32;
        scene
    };
    let middle = at(P);
    assert_eq!(
        middle.nodes.iter().filter(|node| node.glow.level > 0.0).count(),
        1,
        "the fixture must ship exactly one LIT node, or this is a claim about a chord",
    );
    // Non-vacuous: there is a halo in these frames for the exponent to leave
    // alone. Against the same scene with the Reach at 0 — the light's own off
    // switch — rather than against a black frame, since the node's own ink is
    // in both and is not what is being measured.
    let mut dark = at(P);
    dark.glow_reach = 0.0;
    let (lit, unlit) = (shooter.shot(&middle), shooter.shot(&dark));
    assert!(
        total_light(&lit) > total_light(&unlit),
        "the fixture draws no light at all ({} against {}), so every frame below agrees for \
         the wrong reason",
        total_light(&lit),
        total_light(&unlit),
    );

    for p in SWEEP {
        let shot = shooter.shot(&at(p));
        let moved = shot.iter().zip(&lit).filter(|(a, b)| a != b).count();
        let worst = shot.iter().zip(&lit).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        assert_eq!(
            moved, 0,
            "an exponent of {p} moved {moved} channels of a frame with ONE lit node in it, \
             by up to {worst}/255",
        );
    }
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
/// directions, so each could in principle lay down its own strip's colour at a
/// different angle; on THIS fixture they do not, since the strip carries the
/// lit slot's one colour at every angle. Measured over the read column, the
/// two singles agree in every CHANNEL and not merely in the 15829 against
/// 15829 22abb5c3 read off their sum: 4846, 4519 and 6464 apiece. A
/// per-channel norm of two equal readings is the scalar norm of one of them,
/// so the summed brightness is that reading scaled by the norm's own gain,
/// whatever the colour is.
///
/// Read at the Union bar's two ends as well as at the fresh middle
/// ([`SWEEP`]), which is the whole of what the bar is: the same two halos meet
/// 41% over one at the bottom, 9% in the middle and 2% at the top, and the
/// exponent the shader roots by has to be the one it raised by at each.
///
/// The tolerance is 0.5% of one node's light and it is stated against the
/// SMALLEST of the three effects: 2.2% at the top of the bar, so a reading that
/// took the exponent from anywhere else is four times outside it, and the two
/// larger effects are further out again. It is not slack for the claim — the
/// three measurements land 0.06%, 0.05% and 0.03% off — but for a driver.
/// Nothing else here comes close: the screen blend this replaces reads 73% over
/// one node at this geometry (measured), and a plain max reads 0% at every
/// exponent.
#[test]
fn two_halos_meeting_read_the_norm_of_one_rather_than_their_sum() {
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // Two node uv apart, so the bisector stands one uv from each: outside the
    // rings a node draws (0.795 uv) and inside the span its light reaches
    // (0.795 + the Reach), which is what puts a bare pixel in both halos.
    let half = single_marked_node(0, 0).node_radius * 1.8;
    let at = |lit: [f32; 2], p: f64| -> Scene {
        let mut scene = bare_lattice(1.5);
        scene.glow_union = p as f32;
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

    let dark = bisector(&shooter.shot(&at([0.0, 0.0], P)));
    assert_eq!(dark, 0, "the read column is not bare: {dark} of something that is not the light");
    // The two singles are shot ONCE, at the middle of the bar. A single lit node
    // is the same frame at every exponent to the byte — its own term is 1 and
    // the root of 1 is 1, which is
    // `a_lone_halo_is_the_same_frame_at_every_bar_position`'s whole claim — so a
    // sweep of them would be three copies of one shot.
    let left = bisector(&shooter.shot(&at([1.0, 0.0], P)));
    let right = bisector(&shooter.shot(&at([0.0, 1.0], P)));
    assert!(
        left > 0 && right > 0,
        "a node's halo does not reach the bisector at all ({left} and {right}); the fixture \
         measures one light rather than two meeting",
    );

    for p in SWEEP {
        let both = bisector(&shooter.shot(&at([1.0, 1.0], p)));
        let gain = both as f64 / ((left + right) as f64 / 2.0);
        let want = 2f64.powf(1.0 / p);
        assert!(
            (gain - want).abs() < 0.005,
            "at an exponent of {p} two halos meeting read {gain:.4} times one, against the \
             norm's {want:.4} — outside a tolerance of 0.5%, which is set against the 2.2% \
             the top of the bar is worth (one node {left} and {right}, both {both})",
        );
    }
}

/// A CLUSTER does not brighten: `n` nodes over one another read at most
/// `n^(1/p)` times one of them at the same pixel — and, where the `n` coverages
/// are EQUAL as this fixture makes them, exactly that, which is what holds the
/// reading to all four of them rather than to however many happened to ship.
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
    // The same 1% the other way, and that is the fixture's own reach rather
    // than a second reading of the claim: four EQUAL coverages make the norm an
    // equality, so a fixture that shipped three of them (or one) reads
    // `3^(1/p)` or `1`, both of which clear the bound above and neither of
    // which clears this. The measurement lands 0.02% under `want`, so the slack
    // is for a driver.
    assert!(
        gain >= want * 0.99,
        "{N} nodes at one place read {gain:.4} times one, short of the norm's {want:.4}: \
         fewer of them are lighting the ring than the fixture ships",
    );
    assert!(
        gain < screen,
        "{N} nodes read {gain:.4} times one, which the screen blend would have given at a \
         coverage of {coverage:.3} ({screen:.4})",
    );
}

/// A second note never takes light OUT of a channel: where two halos meet,
/// every channel of the pair is at least what either node put there on its
/// own, and is exactly the p-norm of the two — the light's own fold, run a
/// second time on each channel of the nodes' premultiplied colour.
///
/// The property both of #683's rejected colour rules lacked, and the one Yan
/// asked for by name. Weighted by the norm's own terms, `(a_i/m)^p`, a node
/// nearer by a hair took the whole vote and two hues met at a SEAM. Weighted
/// by the plain coverages the colour is a MEAN, and a mean lands between the
/// two colours while the norm holds the light near the larger: where a dim
/// note's halo crossed a bright one's, the pixel kept the bright coverage and
/// got a diluted colour, so adding a note made an area DARKER. The shipped
/// pitch ramp's two ends are 3.9x apart in luminance, and two notes meeting at
/// half coverage apiece read 22% under the brighter one ALONE at a bar reading
/// of +19% and 35% under at the fresh +9%. A p-norm is at least its largest
/// term at every exponent, so a per-channel fold cannot do that to any channel.
///
/// The fixture is `two_halos_meeting_read_the_norm_of_one_rather_than_their_sum`'s
/// with the two nodes given different colours and stood a tenth further apart,
/// read on a column a fifth of a uv to one side of the bisector: far enough
/// over that the nearer node's coverage is well above the farther one's (about
/// 2 to 1), so a mean and a union disagree by most of the distance between the
/// hues; near enough that every pixel of the run still sees both halos; and —
/// with the extra spacing — clear of the nearer node's own rings, which end at
/// 0.795 uv and would otherwise put ink on the column.
///
/// The two hues SHARE channels, `(1, 0.25, 0.25)` against `(0.25, 0.25, 1)`,
/// so all three channels carry two non-trivial terms and the norm is read
/// where it is a norm rather than where it is a copy. A saturated pair would
/// leave green at nothing in both nodes and red and blue with one term each,
/// which is the fixture that passes for the wrong reason: a plain max reads
/// the same there.
///
/// Reach against the fold this replaces: the pair's RED is what separates the
/// two rules, since the nearer node holds almost all of it and a mean gives
/// most of it away. Averaged over the run the mean fold reads red at 148.8,
/// 133.5 and 133.4 over the sweep, against this one's 178.8, 177.5 and 177.5
/// and against the 177.5 the nearer node lights on its own — 29, 44 and 44
/// levels of light taken back out by the second note. Read per pixel, where
/// the floor is, its worst row is 30 levels under at the bottom of the bar and
/// 46 under at the other two, so it misses that floor by 30 to 46 times the
/// slack and the mean by 60 to 88 tolerances. Its blue misses by 1.9
/// tolerances at the two higher exponents and its green passes everywhere; red
/// is the channel that carries this fixture.
#[test]
fn a_second_note_never_takes_light_out_of_any_channel() {
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    /// How far left of the bisector the column stands, in node uv.
    const OFFSET: f32 = 0.2;
    /// Rows read either side of the middle one.
    const RUN: usize = 16;
    /// One 8-bit level, which is what the floor below is allowed to give away:
    /// the final write is dithered, so a channel that should land exactly on
    /// its neighbour's reading can come out a level either side of it. Slack
    /// for that alone — the shortfalls this test is against are tens of levels.
    const SLACK: f64 = 1.0;
    /// How far the pair's channel may stand off the norm of the two singles',
    /// in 8-bit levels, averaged over the run. HALF a level, which is half of
    /// what one pixel's rounding alone is worth: each single is a dithered byte
    /// good to half a level, the norm carries at most their sum through (its
    /// two derivatives are `(l/n)^(p-1)` and `(r/n)^(p-1)`, together at most
    /// `2^(1/p)`), and the pair's own byte adds another half. Averaged over 33
    /// pixels of a screen-fixed dither those cancel: the nine readings land
    /// 0.043 off at worst, a twelfth of this, so it is slack for a driver
    /// rather than for the claim.
    const TOLERANCE: f64 = 0.5;
    // 1.1 uv each side of the bisector, so the column stands 0.9 uv from the
    // nearer node and 1.3 from the farther, both inside the light's 1.595.
    let half = single_marked_node(0, 0).node_radius * 1.8 * 1.1;
    let at = |lit: [f32; 2], p: f64| -> Scene {
        let mut scene = bare_lattice(1.5);
        scene.glow_union = p as f32;
        // Two hues that share every channel: a node's light is its lit pitch's
        // table entry, so the table is cut in two — red over a quarter of the
        // other two up to the middle of it, blue over the same quarter above —
        // and the right node is pitched eleven semitones over the left, across
        // the cut. The fixture's own table is a sweep with 0.4 of green under
        // all of it, and these two pitches on it read 0.09 apart in chroma,
        // which is no pair of hues at all.
        scene.pitch_lut = std::array::from_fn(|k| {
            if k * 2 < harmonigraph_scene::PITCH_LUT_N {
                glam::Vec4::new(1.0, 0.25, 0.25, 1.0)
            } else {
                glam::Vec4::new(0.25, 0.25, 1.0, 1.0)
            }
        });
        let node = scene.nodes[0];
        scene.nodes = [-half, half]
            .iter()
            .enumerate()
            .map(|(i, x)| {
                let mut node = node;
                node.world_pos = glam::Vec3::new(*x, 0.0, 0.0);
                node.lattice_pos = harmonigraph_core::LatticePos::new(i as i32, 0, 0);
                if i == 1 {
                    node.cents = 1100.0;
                }
                node.glow.level = lit[i];
                node
            })
            .collect();
        rows_per_node(&mut scene);
        on_the_middle_pixel(&mut scene);
        scene
    };
    let column = (CENTRE.x - OFFSET * pixels_per_uv(&at([1.0, 1.0], P))).floor() as usize;
    let run = |shot: &[u8]| -> Vec<[f64; 3]> {
        (SIZE[1] as usize / 2 - RUN..=SIZE[1] as usize / 2 + RUN)
            .map(|row| {
                let px = (row * SIZE[0] as usize + column) * 4;
                [f64::from(shot[px]), f64::from(shot[px + 1]), f64::from(shot[px + 2])]
            })
            .collect()
    };
    // Per CHANNEL and not per pixel: what the norm below is read on is each
    // channel's own two terms, and a channel one node leaves at nothing is a
    // copy of the other rather than a fold of both.
    let lit_throughout = |pixels: &[[f64; 3]]| pixels.iter().flatten().all(|v| *v > 0.0);
    let means = |pixels: &[[f64; 3]]| -> [f64; 3] {
        pixels.iter().fold([0.0; 3], |mut acc, px| {
            for (a, v) in acc.iter_mut().zip(px) {
                *a += v / pixels.len() as f64;
            }
            acc
        })
    };

    let dark: f64 = run(&shooter.shot(&at([0.0, 0.0], P))).iter().flatten().sum();
    assert_eq!(dark, 0.0, "the read column is not bare: {dark} of something that is not the light");
    let left = run(&shooter.shot(&at([1.0, 0.0], P)));
    let right = run(&shooter.shot(&at([0.0, 1.0], P)));
    assert!(
        lit_throughout(&left) && lit_throughout(&right),
        "a channel of the run gets no light from one of the nodes, so the column is outside its \
         halo or that channel carries one term rather than two, and the norm below is read where \
         it is a copy",
    );
    // The two singles have to DIFFER, or the floor below is the pair's own
    // reading and every fold clears it.
    let apart = left
        .iter()
        .zip(&right)
        .flat_map(|(l, r)| l.iter().zip(r).map(|(x, y)| (x - y).abs()))
        .fold(0.0, f64::max);
    assert!(
        apart > 64.0,
        "the two nodes light the run almost identically (no channel more than {apart:.0}/255 \
         apart), so a fold that drops either of them still clears the floor below",
    );

    let (left_mean, right_mean) = (means(&left), means(&right));

    for p in SWEEP {
        let both = run(&shooter.shot(&at([1.0, 1.0], p)));
        // THE CLAIM, per pixel and per channel: nothing the second note does
        // takes light out of what the first one laid down.
        for (row, ((pair, l), r)) in both.iter().zip(&left).zip(&right).enumerate() {
            for (ch, ((got, one), other)) in pair.iter().zip(l).zip(r).enumerate() {
                let floor = one.max(*other);
                assert!(
                    *got >= floor - SLACK,
                    "at an exponent of {p}, row {row} of the run reads {got} in channel {ch} \
                     with both notes lit, under the {floor} one of them puts there alone \
                     ({l:?} and {r:?} meeting as {pair:?})",
                );
            }
        }
        let pair_mean = means(&both);
        // THE VALUE, which is what makes the claim above non-vacuous: the pair
        // is not merely at least the larger of the two, it is the p-norm of
        // them — read off the singles' own bytes, so no coverage, no colour and
        // no clamp is left in the comparison.
        let mut norm = [0.0; 3];
        for (l, r) in left.iter().zip(&right) {
            for ((n, one), other) in norm.iter_mut().zip(l).zip(r) {
                *n += (one.powf(p) + other.powf(p)).powf(1.0 / p) / both.len() as f64;
            }
        }
        for (ch, (got, want)) in pair_mean.iter().zip(norm).enumerate() {
            assert!(
                (got - want).abs() < TOLERANCE,
                "at an exponent of {p} channel {ch} averages {got:.2}/255 over the run, against \
                 the {want:.2} the two singles' own readings union to (one note averages {:.2} \
                 and {:.2})",
                left_mean[ch],
                right_mean[ch],
            );
        }
    }
}
