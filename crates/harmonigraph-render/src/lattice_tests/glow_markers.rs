//! The shadow a resting cross casts from its own ink, multiplied into
//! everything already in the frame under it.

use super::fixtures::*;
use crate::*;

fn with_shadow_kernel(mut scene: Scene, kernel: harmonigraph_scene::ShadowKernel) -> Scene {
    for style in scene.shadow.groups_mut() {
        style.kernel = kernel;
    }
    scene
}

/// Which pixels of `ground` the markers DARKEN, against the same frame with no
/// marker in it.
///
/// `ground` is the set the markers' ink never reaches, taken from the pair at a
/// depth of 0 where a marker writes ink and nothing else. It is the ink's own
/// footprint that is being excluded and the footprint does not move with the
/// depth, so one reading of it answers for every shot here.
fn shadowed_ground(
    shooter: &mut Shooter,
    shadow: f32,
    taper_start: f32,
) -> (Vec<usize>, std::collections::BTreeSet<usize>) {
    let flat_bare = shooter.shot(&{
        let mut s = shadowed_markers(0.0, shadow, taper_start);
        s.pluses.clear();
        s
    });
    let flat = shooter.shot(&shadowed_markers(0.0, shadow, taper_start));
    let ground: Vec<usize> =
        (0..flat.len()).step_by(4).filter(|&i| flat[i..i + 4] == flat_bare[i..i + 4]).collect();
    let deep_bare = shooter.shot(&{
        let mut s = shadowed_markers(1.0, shadow, taper_start);
        s.pluses.clear();
        s
    });
    let deep = shooter.shot(&shadowed_markers(1.0, shadow, taper_start));
    let dimmed: std::collections::BTreeSet<usize> = ground
        .iter()
        .copied()
        .filter(|&i| brightness(&deep[i..i + 3]) < brightness(&deep_bare[i..i + 3]))
        .collect();
    // The pair at a depth of 0 is the other half of every claim below: at a
    // depth of 0 a marker's draw multiplies by exactly 1, and nothing else it
    // does can subtract.
    let flat_dimmed = ground
        .iter()
        .filter(|&&i| brightness(&flat[i..i + 3]) < brightness(&flat_bare[i..i + 3]))
        .count();
    assert_eq!(flat_dimmed, 0, "a marker took light off the ground at a Shadow depth of 0");
    (ground, dimmed)
}

/// A marker darkens a NODE's halo around its cross, on the Shadow bars the
/// node's own rings cast by.
///
/// The whole claim of the feature: the light is composited under everything, so
/// what a marker's shadow lands on is the melded field, which is somebody else's
/// light entirely. A shadow that never reached past the cross would mean the
/// marker's own draw was multiplying nothing but the pixels its ink already
/// covers.
///
/// The pair at a depth of 0 is asserted inside [`shadowed_ground`]: at that
/// depth nothing here subtracts, which is what says the darkening measured is
/// the shadow and not the marker draw finding some other way to take light off
/// the picture.
#[test]
fn a_marker_holds_a_nodes_halo_off_its_own_cross() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let (ground, dimmed) = shadowed_ground(&mut shooter, 0.8, 1.0);
    assert!(
        ground.len() > 1000,
        "the fixture must leave ground for the shadow to land on, not {}",
        ground.len(),
    );
    assert!(
        dimmed.len() > 200,
        "a full Shadow depth darkened {} of the {} pixels the markers' ink never reaches",
        dimmed.len(),
        ground.len(),
    );
}

/// A cross's shadow is worth its ink: half a marker takes half the light off
/// the halo it stands in, and a marker that is not there takes none.
///
/// This is the whole of what a marker hands the picture — one number, spent on
/// the ink and on the share of the shadow alike (`PlusInstance::strength`) —
/// and it is what a position handing itself back to a name looks like: the
/// cross grows and the shadow under it grows on the same clock. A shadow closed
/// against the LIGHT instead runs on the Glow release, so a cross fully back
/// would stand with nothing under it for as long as the halo it stands in takes
/// to leave.
///
/// Measured on the light TAKEN rather than on a pixel count, which is what
/// makes the middle of the ramp a claim rather than a rounding: a shallower
/// shadow lightens the pixels it bites before it stops biting them, and an
/// 8-bit count sees the second and not the first. A marker's opacity puts NO
/// light into these pixels, so the only thing it can do out there is take some
/// away.
///
/// The excluded footprint is read at full ink, where it is largest — a fainter
/// marker inks a subset of it — so one ground set answers for every shot.
///
/// The marker stands OUT in the node's halo and not at the node's own centre,
/// which is `shadowed_markers`' delicate number and the one thing this fixture
/// cannot do without: shadows MULTIPLY, so a cross standing inside the node's
/// own shadow is read on pixels the node has already darkened, and half a
/// cross's share of what is left is not half of what a whole one takes.
#[test]
fn a_crosss_shadow_is_worth_its_ink() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // One marker out in the node's halo, in a frame whose only light is that
    // node's, `shadowed_markers` putting no other light in the frame.
    let at = |strength: f32, depth: f32| -> Scene {
        let mut scene = shadowed_markers(depth, 0.8, 1.0);
        scene.pluses =
            vec![one_marker(glam::Vec3::new(2.6, 0.0, 0.0), 0.4, scene.lattice_ground, strength)];
        scene
    };
    let bare = |shooter: &mut Shooter, depth: f32| {
        shooter.shot(&{
            let mut s = at(0.0, depth);
            s.pluses.clear();
            s
        })
    };

    // The ink's own footprint, and everything it reaches excluded from the
    // counts below: what is being measured is light the cross took off the
    // picture around it, not the cross itself.
    let flat_bare = bare(&mut shooter, 0.0);
    let flat = shooter.shot(&at(1.0, 0.0));
    let ground: Vec<usize> =
        (0..flat.len()).step_by(4).filter(|&i| flat[i..i + 4] == flat_bare[i..i + 4]).collect();
    assert!(
        ground.len() > 1000,
        "the fixture must leave halo for a bite to land in, not {}",
        ground.len(),
    );

    let deep_bare = bare(&mut shooter, 1.0);
    let taken = |frame: &[u8]| -> i64 {
        ground
            .iter()
            .map(|&i| (brightness(&deep_bare[i..i + 3]) - brightness(&frame[i..i + 3])).max(0))
            .sum()
    };
    let whole = taken(&shooter.shot(&at(1.0, 1.0)));
    let half = taken(&shooter.shot(&at(0.5, 1.0)));
    let none = taken(&shooter.shot(&at(0.0, 1.0)));
    assert!(
        whole > 0,
        "a whole cross took no light off the halo it stands in — with none there is nothing \
         for the rest of this to be a share of",
    );
    assert!(
        half > 0 && half < whole,
        "half a cross took {half} of the {whole} a whole one takes, which is not a share of it",
    );
    assert_eq!(none, 0, "a marker with no ink still took {none} of light off the halo");
}

/// A marker's shadow finishes with the taper of its arm: adding a fade brings
/// the shadow's last visible pixel inward, and a deeper fade never pushes it
/// back out.
///
/// Read along the arm rather than as a pixel count, and that is what makes it a
/// claim about the SHAPE: a count also moves when the taper moves the ink's own
/// footprint out of the ground being counted, so two counts can differ with the
/// shadow's reach unchanged. Each cross is walked against its OWN depth-0 frame,
/// which is what cancels that ink; what is left is the shadow alone.
#[test]
fn a_markers_shadow_fades_out_with_the_taper_of_its_arm() {
    const SIZE: [u32; 2] = [256, 256];
    // `one_shadow_is_one_distance_whatever_the_cross_it_stands_off`'s numbers,
    // and for its reasons: an arm wide enough on screen to rule a shadow by,
    // and a Shadow that finishes well inside the halo doing the ruling.
    const SHADOW: f32 = 0.30;
    const ARM: f32 = 0.9;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    for kernel in
        [harmonigraph_scene::ShadowKernel::Gaussian, harmonigraph_scene::ShadowKernel::Distance]
    {
        let ruler = with_shadow_kernel(lone_tapered_marker(ARM, SHADOW, 0.0, 1.0), kernel);
        let centre = on_screen(&ruler, SIZE, glam::Vec3::new(LONE_OFFSET, 0.0, 0.0));
        let tip = on_screen(&ruler, SIZE, glam::Vec3::new(LONE_OFFSET + ARM, 0.0, 0.0));
        let arm = tip - centre;
        let side = glam::Vec2::new(-arm.y, arm.x).normalize()
            * (arm.length() * ruler.plus_half_width + 2.0);
        let row = (centre + side).y.round() as usize;
        let mid = centre.x.round() as usize;
        let at = |buf: &[u8], x: usize| -> i64 {
            let i = (row * SIZE[0] as usize + x) * 4;
            brightness(&buf[i..i + 3])
        };

        // The outermost column just outside the arm's side where the shadow
        // reads darker than the same frame without one. Rightward because the
        // node lighting the frame is to the LEFT: that half holds no other ink
        // and no other shadow.
        let reach = |shooter: &mut Shooter, arm: f32, taper: f32| -> usize {
            let cold = shooter
                .shot(&with_shadow_kernel(lone_tapered_marker(arm, SHADOW, 0.0, taper), kernel));
            let hot = shooter
                .shot(&with_shadow_kernel(lone_tapered_marker(arm, SHADOW, 1.0, taper), kernel));
            let mut out = mid;
            for x in mid..SIZE[0] as usize {
                if at(&hot, x) < at(&cold, x) {
                    out = x;
                }
            }
            out
        };
        let square = reach(&mut shooter, ARM, 1.0);
        let some = reach(&mut shooter, ARM, 0.6);
        let most = reach(&mut shooter, ARM, 0.25);
        assert!(
            square > mid,
            "{kernel:?}: the square-ended cross cast no shadow past its own crossing",
        );
        assert!(
            square > some && some >= most && most > mid,
            "{kernel:?}: arms fading over none, two fifths and three quarters of their length \
             shadowed out to {square}, {some} and {most} from a crossing at {mid}",
        );
    }
}

/// A tapered arm gives up its shadow with its INK, and with nothing else: the
/// deeper the taper, the less light the cross takes off the halo, and never in
/// one step.
///
/// An arm's ink fades to nothing over its last stretch so that it arrives at
/// nothing rather than stopping at something (`plus_coverage`), and the shadow
/// takes that same ramp outside the arm. A taper running through four fifths of
/// the arm still leaves a measurable shadow from the visible inner cross; it
/// does not delete the caster whole.
///
/// The steps are a tenth of the square's apart rather than merely ordered: an
/// order alone is met by one code value, which is what a taper that moved the
/// ink and left the blur behind would give.
///
/// Measured as light TAKEN off the halo rather than as a reach, because what
/// moves here is the depth and not the footprint — the test above is the one
/// that holds the footprint still. The ground is the square end's, which is the
/// largest ink of the family: a tapered cross draws inside that same box at a
/// lower alpha, so one exclusion answers for all of them.
#[test]
fn a_tapered_arm_gives_up_its_shadow_with_the_ink_the_taper_takes() {
    const SIZE: [u32; 2] = [256, 256];
    const SHADOW: f32 = 0.30;
    const ARM: f32 = 0.9;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    for kernel in
        [harmonigraph_scene::ShadowKernel::Gaussian, harmonigraph_scene::ShadowKernel::Distance]
    {
        let bare = |shooter: &mut Shooter, depth: f32| {
            shooter.shot(&{
                let mut s =
                    with_shadow_kernel(lone_tapered_marker(ARM, SHADOW, depth, 1.0), kernel);
                s.pluses.clear();
                s
            })
        };
        let flat_bare = bare(&mut shooter, 0.0);
        let flat =
            shooter.shot(&with_shadow_kernel(lone_tapered_marker(ARM, SHADOW, 0.0, 1.0), kernel));
        let ground: Vec<usize> =
            (0..flat.len()).step_by(4).filter(|&i| flat[i..i + 4] == flat_bare[i..i + 4]).collect();
        assert!(
            ground.len() > 1000,
            "{kernel:?}: the fixture must leave halo for a bite to land in, not {}",
            ground.len(),
        );
        let deep_bare = bare(&mut shooter, 1.0);
        let taken = |shooter: &mut Shooter, taper: f32| -> i64 {
            let frame = shooter
                .shot(&with_shadow_kernel(lone_tapered_marker(ARM, SHADOW, 1.0, taper), kernel));
            ground
                .iter()
                .map(|&i| (brightness(&deep_bare[i..i + 3]) - brightness(&frame[i..i + 3])).max(0))
                .sum()
        };
        let square = taken(&mut shooter, 1.0);
        let some = taken(&mut shooter, 0.6);
        let most = taken(&mut shooter, 0.2);
        assert!(
            square > 0,
            "{kernel:?}: a square-ended cross took no light off the halo it stands in",
        );
        assert!(
            some * 10 < square * 9 && most * 10 < some * 9,
            "{kernel:?}: an arm fading over none, two fifths and four fifths of its length took \
             {square}, {some} and {most} of light off the halo, which is not one order by a \
             tenth a step",
        );
        assert!(
            most * 100 > square * 10,
            "{kernel:?}: an arm fading from a fifth of its length took {most} against the square \
             end's {square}, which leaves no measurable shadow from the visible inner cross",
        );
    }
}

/// A marker's shadow neither shows through nor outlines the share of its arm
/// the taper makes transparent.
///
/// The arm's full box is the marker's own surface even where its ink is
/// fading, so its shadow belongs only outside that box. The shadow outside the
/// sides gives up its depth on the same ramp, or the transparent tip remains
/// visible as a hollow silhouette. The tapered pixels are identified from the
/// difference against a square-ended copy rather than from coordinates: that
/// proves the fixture reaches the alpha ramp, and a grey pane makes any shadow
/// leaking through it visible as lost light.
#[test]
fn a_markers_shadow_does_not_show_through_its_tapered_arm() {
    const SIZE: [u32; 2] = [256, 256];
    const SHADOW: f32 = 0.30;
    const ARM: f32 = 0.9;
    const TAPERED: f32 = 0.2;
    const GROUND: f64 = 0.8;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    for kernel in
        [harmonigraph_scene::ShadowKernel::Gaussian, harmonigraph_scene::ShadowKernel::Distance]
    {
        let scene = |depth: f32, taper: f32| {
            let mut scene =
                with_shadow_kernel(lone_tapered_marker(ARM, SHADOW, depth, taper), kernel);
            scene.nodes.clear();
            scene.glow_reach = 0.0;
            scene
        };
        let bare = shooter.shot(&{
            let mut scene = scene(0.0, 1.0);
            scene.pluses.clear();
            scene
        });
        let square = shooter.shot(&scene(0.0, 1.0));
        let tapered = shooter.shot(&scene(0.0, TAPERED));
        let body: std::collections::BTreeSet<usize> =
            fully_inked(&bare, &square).into_iter().collect();
        let row = SIZE[0] as usize * 4;
        let fading: Vec<usize> = body
            .iter()
            .copied()
            .filter(|&i| {
                i >= row + 4
                    && i + row + 4 < square.len()
                    && body.contains(&(i - 4))
                    && body.contains(&(i + 4))
                    && body.contains(&(i - row))
                    && body.contains(&(i + row))
            })
            .filter(|&i| tapered[i..i + 4] != square[i..i + 4])
            .collect();
        assert!(
            fading.len() > 200,
            "{kernel:?}: only {} fully covered arm pixels reach the taper",
            fading.len(),
        );

        shooter.clear = wgpu::Color { r: GROUND, g: GROUND, b: GROUND, a: 1.0 };
        let flat_bare = shooter.shot(&{
            let mut scene = scene(0.0, 1.0);
            scene.pluses.clear();
            scene
        });
        let flat_square = shooter.shot(&scene(0.0, 1.0));
        let flat = shooter.shot(&scene(0.0, TAPERED));
        let deep = shooter.shot(&scene(1.0, TAPERED));

        let moved = fading
            .iter()
            .filter(|&&i| (0..4).any(|c| deep[i + c].abs_diff(flat[i + c]) > 1))
            .count();
        let worst = fading
            .iter()
            .map(|&i| (0..4).map(|c| deep[i + c].abs_diff(flat[i + c])).max().unwrap())
            .max()
            .unwrap_or(0);
        assert_eq!(
            moved,
            0,
            "{kernel:?}: the Shadow depth moved {moved} of the marker's {} fading arm pixels, \
             by up to {worst}",
            fading.len(),
        );

        let measured = scene(0.0, TAPERED);
        let centre = on_screen(&measured, SIZE, glam::Vec3::new(LONE_OFFSET, 0.0, 0.0));
        let tip = on_screen(&measured, SIZE, glam::Vec3::new(LONE_OFFSET + ARM, 0.0, 0.0));
        let arm = tip - centre;
        let side = glam::Vec2::new(-arm.y, arm.x).normalize()
            * (arm.length() * measured.plus_half_width + 2.0);
        let loss_at = |along: f32| {
            let p = centre + arm * along + side;
            let i = ((p.y.round() as usize * SIZE[0] as usize + p.x.round() as usize) * 4) as usize;
            brightness(&flat[i..i + 3]) - brightness(&deep[i..i + 3])
        };
        let body_loss = loss_at(0.45);
        let tip_loss = loss_at(0.97);
        assert!(
            body_loss > 20,
            "{kernel:?}: the shadow beside the fading arm took only {body_loss} of light",
        );
        assert!(
            tip_loss <= 1,
            "{kernel:?}: the shadow still outlines the transparent arm tip by {tip_loss}, \
             against {body_loss} beside the visible arm",
        );

        let ground: Vec<usize> = (0..flat_square.len())
            .step_by(4)
            .filter(|&i| flat_square[i..i + 4] == flat_bare[i..i + 4])
            .collect();
        let dimmed = ground
            .iter()
            .filter(|&&i| brightness(&deep[i..i + 3]) < brightness(&flat[i..i + 3]))
            .count();
        assert!(
            dimmed > 200,
            "{kernel:?}: the marker darkened only {dimmed} pixels outside its arm",
        );
        shooter.clear = wgpu::Color::BLACK;
    }
}

/// One Shadow is ONE distance: what a marker's shadow reaches past the ink casting
/// it is a world length off the bar, not a share of the cross.
///
/// This is the whole of what sharing the node's Shadow bar buys, and it is a
/// claim no relative measurement can hold. σ is a length on the PANE
/// (`shadow::sigma_px`), taken off the home node's radius; a σ scaled by each
/// caster's own size instead would leave every other shadow test here passing,
/// each being monotone in the Shadow, a superset, or a comparison between two
/// arms of one length. What it changes is that the lattice's Shadow becomes as
/// many distances as there are marker sizes; only a frame holding two different
/// arms can see it, and only against a world ruler.
///
/// The ruler is the cross's own INK: a square-ended arm draws out to exactly its
/// own length, so the longer marker's two arm tips are a known number of world
/// units apart and every length below is read through that. Its own ink and not
/// the halo it stands in, which reaches the pane's every corner and has no edge
/// to measure — and an edge on a decay would be a threshold where this is a
/// footprint. The shadow's own edge IS such a threshold (a Gaussian never
/// reaches zero), which is why the claim is a DIFFERENCE between two arms under
/// one threshold rather than either arm's reach against the Shadow.
#[test]
fn one_shadow_is_one_distance_whatever_the_cross_it_stands_off() {
    const SIZE: [u32; 2] = [256, 256];
    // Narrow enough that the longer arm's shadow finishes well inside the halo
    // ruling it, which is a condition on the READING and not on the claim: the
    // edge below is where a difference crosses the 8-bit floor, and out where
    // the light is already dim the deeper of two shadows crosses it early, so
    // two reaches that differ by an arm's length are measured as differing by
    // less.
    const SHADOW: f32 = 0.30;
    const SHORT: f32 = 0.35;
    const LONG: f32 = 0.9;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let row = SIZE[1] as usize / 2;
    let at = |buf: &[u8], x: usize| -> i64 {
        let i = (row * SIZE[0] as usize + x) * 4;
        brightness(&buf[i..i + 3])
    };
    // The columns on the centre row where `marked` differs from `bare` at all:
    // one contiguous run per shot here, the frame holding one cross and nothing
    // else to the node's right.
    let span = |marked: &[u8], bare: &[u8]| -> (usize, usize) {
        let cols: Vec<usize> = (0..SIZE[0] as usize)
            .filter(|&x| {
                let i = (row * SIZE[0] as usize + x) * 4;
                marked[i..i + 4] != bare[i..i + 4]
            })
            .collect();
        (*cols.first().unwrap(), *cols.last().unwrap())
    };

    // The ruler: the long cross's own ink, against a frame with no cross in it.
    // Its arms run 2 * LONG world tip to tip along this row.
    let bare = shooter.shot(&{
        let mut s = lone_shadowed_marker(LONG, SHADOW, 0.0);
        s.pluses.clear();
        s
    });
    let flat_long = shooter.shot(&lone_shadowed_marker(LONG, SHADOW, 0.0));
    let (left, right) = span(&flat_long, &bare);
    let per_world = (right - left) as f32 / (LONG * 2.0);
    assert!(
        per_world > 8.0,
        "the cross has to be drawn wide enough to rule a shadow by, not {per_world:.1}px per world",
    );
    // Its own centre, so the walk below starts at the crossing and goes out.
    let mid = left.midpoint(right);

    // The outermost column right of the crossing where `hot` reads darker than
    // `cold`. Rightward because the node lighting the frame is to the LEFT: the
    // half being walked holds no other ink and no other shadow.
    let edge = |hot: &[u8], cold: &[u8]| -> usize {
        let mut out = mid;
        for x in mid..SIZE[0] as usize {
            if at(hot, x) < at(cold, x) {
                out = x;
            }
        }
        out
    };

    let flat_short = shooter.shot(&lone_shadowed_marker(SHORT, SHADOW, 0.0));
    let short_reach = edge(&shooter.shot(&lone_shadowed_marker(SHORT, SHADOW, 1.0)), &flat_short);
    let long_reach = edge(&shooter.shot(&lone_shadowed_marker(LONG, SHADOW, 1.0)), &flat_long);
    assert!(
        long_reach > short_reach && short_reach > mid,
        "both arms must cast a shadow, the longer one further: {short_reach} and {long_reach}",
    );

    // What the longer cross buys is its own extra ink and nothing else: the Shadow
    // past the ink is one distance, so the two shadows differ by exactly the
    // two arms' difference.
    let grew = (long_reach - short_reach) as f32 / per_world;
    let want = LONG - SHORT;
    assert!(
        (grew - want).abs() < 0.12,
        "an arm {} longer pushed its shadow {grew:.2} further ({short_reach}px to {long_reach}px \
         at {per_world:.1}px per world) — the Shadow is being read as a share of the ink",
        want,
    );
}

/// A RESTING MARKER wears the wash, out of the same field a node's ink takes it
/// from.
///
/// The marker field is drawn over ground the light is already under, so unwashed
/// a marker inside a halo would be the one thing in the picture that gets darker
/// the more light stands at it — flat ground laid over lit ground — and the
/// lattice would read as a field of holes punched exactly where the light is
/// brightest. That is what the wash closes here.
///
/// Two claims, on the marker's own pixels:
///
/// - The light lifts more than half of them. That claim is also what makes
///   the fixture honest — nothing lifts where no light reaches, so a marker
///   parked outside the halo fails here rather than passing vacuously.
/// - Nothing is ever dimmed, the wash being a screen (`wash_over`).
///
/// On the pixels the marker covers in FULL ([`fully_inked`]): what shows
/// through its antialiased rim is the ground's share of the light, which is the
/// Shadow bars' answer and not this bar's.
#[test]
fn a_resting_marker_wears_the_wash_it_stands_in() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // The lit node at the origin, and one marker beside it. uv 1 is 1.8 node
    // radii (`node_vertex`), so this node's outermost ring at uv 0.795 reaches
    // 1.57 world units and the marker's near tip stands at 2.2 — clear of the
    // ink, and inside a reach dialled to carry light well past it.
    let at = |reach: f32, marker: bool| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        if marker {
            scene.pluses =
                vec![one_marker(glam::Vec3::new(3.0, 0.0, 0.0), 0.8, scene.lattice_ground, 1.0)];
        }
        scene
    };
    let bare = shooter.shot(&at(0.0, false));
    // The glow OFF is what the marker is measured against: no light at the
    // pixel is the one setting left that takes the wash out of the picture.
    let off = shooter.shot(&at(0.0, true));
    let marker = fully_inked(&bare, &off);
    assert!(
        marker.len() > 300,
        "the marker covers {} pixels the node had not already covered",
        marker.len(),
    );

    let worn = shooter.shot(&at(1.6, true));
    let lifted = marker
        .iter()
        .filter(|&&i| brightness(&worn[i..i + 3]) > brightness(&off[i..i + 3]))
        .count();
    assert!(
        lifted * 2 > marker.len(),
        "the light lifted {lifted} of the marker's {} pixels: the marker is not wearing the \
         light it stands in",
        marker.len(),
    );
    let dimmed = marker.iter().filter(|&&i| (0..3).any(|c| worn[i + c] < off[i + c])).count();
    assert_eq!(
        dimmed,
        0,
        "the wash took light off {dimmed} of the marker's {} pixels",
        marker.len(),
    );
    // The furthest any one channel of the marker moves between the two shots,
    // which is what the fixture's own non-vacuity is read in.
    let spread = |a: &[u8], b: &[u8]| {
        marker
            .iter()
            .map(|&i| (0..3).map(|c| a[i + c].abs_diff(b[i + c])).max().unwrap())
            .max()
            .unwrap()
    };
    let by_wash = spread(&worn, &off);
    assert!(
        by_wash > 20,
        "the fixture's wash moves the marker by {by_wash}; there is nothing here to measure",
    );
}
