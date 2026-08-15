//! The drawn window: what [`ViewConfig::scrolled`] builds out of a camera and
//! a pane's aspect, and the two things it has to be at once — wide enough that
//! nothing on screen is missing from it, and tight enough that the lattice
//! being unbounded costs a bounded amount of work.

use crate::*;
use glam::{Vec2, Vec3};
use harmonigraph_core::coords;

/// Aspects worth sweeping: a tall pane, a square one, the docked pane's rough
/// shape, and the two render frames (16:9 and a wide one).
const ASPECTS: [f32; 5] = [0.6, 1.0, 1.5, 16.0 / 9.0, 2.4];

const PROJECTIONS: [Projection; 3] =
    [Projection::Perspective, Projection::Orthographic, Projection::Cabinet];

/// Where a position lands on a pane of `aspect`, in NDC — inside the viewport
/// is `|x| <= 1 && |y| <= 1` — or `None` behind the camera.
///
/// `view` must be the one the scene is DERIVED from, because the placement is
/// relative to its center (`derive_scene` draws at `(pos - center) * spacing`)
/// and passing a different one silently moves every node. A coverage test that
/// projected through the reach view while the renderer drew through the
/// scrolled one had both sides of its assertion carrying the same offset,
/// which cancelled — and it passed over a picture with sixty-four holes in it.
fn ndc(camera: &Camera, view: &ViewConfig, aspect: f32, pos: LatticePos) -> Option<Vec2> {
    let world = crate::lattice_to_world(pos - view.center(), view.spacing);
    let clip = camera.view_proj(aspect) * world.extend(1.0);
    (clip.w > 0.0).then(|| Vec2::new(clip.x / clip.w, clip.y / clip.w))
}

/// Every node the pane actually shows is in the window that was built for it.
///
/// This is the half that cannot be traded away. A window too wide costs
/// instances; a window too narrow is a hole in the picture — the lattice
/// stopping in mid-air part way across the pane — and it would appear only at
/// the aspect and zoom that produced it, which is exactly the bug a swept test
/// is for.
///
/// Checked by projecting each candidate through the SAME matrix the renderer
/// draws with, rather than by restating the window's own arithmetic, so a
/// mistake in that arithmetic cannot agree with itself.
///
/// Cabinet is swept unconditionally and the other two are not, which is the
/// [`MAX_DRAWN_NODES`] split rather than a gap in the sweep: a tilted camera
/// can put an unbounded span of sheet on screen, so the cap is allowed to
/// clip it, and a test demanding otherwise would be demanding the impossible.
/// What the cap must never do is clip a cabinet picture — see
/// [`a_cabinet_camera_never_reaches_the_node_budget`].
#[test]
fn everything_the_pane_shows_is_in_the_window() {
    let view = ViewConfig { extent_sevens: 2, ..ViewConfig::default() };
    for projection in PROJECTIONS {
        for aspect in ASPECTS {
            for distance in [Camera::MIN_DISTANCE, Camera::DEFAULT_DISTANCE, Camera::MAX_DISTANCE]
            {
                let camera = Camera { projection, distance, ..Camera::default() };
                let drawn = view.scrolled(&camera, aspect);
                if projection != Projection::Cabinet
                    && drawn.visible_count() > MAX_DRAWN_NODES - 512
                {
                    continue;
                }
                let drawn_set: std::collections::HashSet<_> = drawn.visible_positions().collect();
                // Wider than any window this camera can produce, so the search
                // is not the window's own claim about itself.
                for pos in coords::positions_within(-90..=90, -90..=90, -2..=2) {
                    let Some(p) = ndc(&camera, &drawn, aspect, pos) else {
                        continue;
                    };
                    if p.x.abs() > 1.0 || p.y.abs() > 1.0 {
                        continue;
                    }
                    assert!(
                        drawn_set.contains(&pos),
                        "{projection:?} at aspect {aspect}, distance {distance}: {pos:?} lands \
                         on the pane at {p:?} and is not in the drawn window \
                         ({}x{} about {:?})",
                        drawn.extent_threes,
                        drawn.extent_fives,
                        drawn.center(),
                    );
                }
            }
        }
    }
}

/// Every node a cabinet pane draws is one the analyzer can name: the drawn
/// window never leaves the naming reach, at any zoom, up to a 16:9 frame.
///
/// The two windows answer different questions and are sized apart — the drawn
/// one per pane from the camera, the reach once for the whole UI — so nothing
/// structural keeps them in step, and where they part company the picture and
/// the analyzer contradict each other: a node lit on the lattice while the
/// analyzer draws its off-lattice band for the same pitch and the Notes pane
/// shows no node at all. The reach's default is sized on exactly this, which
/// is why it is checked rather than assumed.
#[test]
fn a_cabinet_pane_never_draws_a_node_the_reach_cannot_name() {
    let view = ViewConfig::default();
    let reach: std::collections::HashSet<_> = view.visible_positions().collect();
    for aspect in [0.6, 1.0, 1.5, 16.0 / 9.0] {
        for distance in [Camera::MIN_DISTANCE, Camera::DEFAULT_DISTANCE, Camera::MAX_DISTANCE] {
            for cabinet_scale in [0.1, 0.6, 1.0] {
                let camera = Camera {
                    projection: Projection::Cabinet,
                    distance,
                    cabinet_scale,
                    ..Camera::default()
                };
                let drawn = view.scrolled(&camera, aspect);
                let outside = drawn
                    .visible_positions()
                    .filter(|&pos| {
                        ndc(&camera, &drawn, aspect, pos)
                            .is_some_and(|p| p.x.abs() <= 1.0 && p.y.abs() <= 1.0)
                    })
                    .filter(|pos| !reach.contains(pos))
                    .count();
                assert_eq!(
                    outside, 0,
                    "aspect {aspect}, distance {distance}, shear {cabinet_scale}: {outside} \
                     nodes are drawn on the pane and cannot be named",
                );
            }
        }
    }
}

/// No cabinet camera reaches the node budget, at any pane, zoom, depth or
/// shear — so under the projection this feature is FOR, the cap is a thing
/// that exists rather than a thing that happens.
///
/// It holds because cabinet faces the sheet by construction: the orbit is
/// ignored, so there is no pitch that lays the lattice edge-on and no camera
/// whose honest window is unbounded. The other two projections have both, and
/// this is the guarantee they cannot give.
#[test]
fn a_cabinet_camera_never_reaches_the_node_budget() {
    let mut worst = 0;
    for sevens in 0..=4 {
        let view = ViewConfig { extent_sevens: sevens, ..ViewConfig::default() };
        for aspect in [0.3, 1.0, 1.5, 16.0 / 9.0, 2.4, 3.0] {
            // Every pitch, to say that cabinet ignores it, and both ends of
            // the shear, which is the one thing that does widen the window.
            for pitch in [0.0, 0.3, Camera::PITCH_LIMIT] {
                for cabinet_scale in [0.1, 0.6, 1.0] {
                    let camera = Camera {
                        projection: Projection::Cabinet,
                        pitch,
                        cabinet_scale,
                        distance: Camera::MAX_DISTANCE,
                        ..Camera::default()
                    };
                    let count = view.scrolled(&camera, aspect).visible_count();
                    assert!(
                        count < MAX_DRAWN_NODES,
                        "a cabinet pane at aspect {aspect}, {sevens} sheets deep, shear \
                         {cabinet_scale} asked for {count} nodes and was trimmed",
                    );
                    worst = worst.max(count);
                }
            }
        }
    }
    // The cap is above the worst cabinet window rather than far above it: a
    // headroom that has quietly become 10x is a cap that has stopped being
    // measured against anything.
    assert!(
        worst * 2 > MAX_DRAWN_NODES,
        "the worst cabinet window is {worst} against a cap of {MAX_DRAWN_NODES}, which is no \
         longer sized on it",
    );
}

/// And the other half: the window is not much wider than the pane. A window
/// that simply returned everything would pass the test above.
///
/// The slack is named in STEPS rather than as a ratio because that is what the
/// margin is: a node's own radius, and a step for the name drawn beside it.
/// Four steps of ring around what shows is comfortably more than the margin
/// asks for and far less than a window that has stopped tracking the pane.
///
/// Cabinet only, and that is the shape of the thing rather than a gap: the
/// window is sized from the world origin, because that is where the block it
/// describes is drawn from, so a camera whose view is NOT centered there
/// spends its extent reaching back to the origin. Cabinet faces the sheet and
/// is centered; a tilted camera is not, and the width it then asks for is the
/// honest cost of covering the pane, not slack.
#[test]
fn the_window_is_not_much_wider_than_the_pane() {
    let view = ViewConfig::default();
    for aspect in ASPECTS {
        for distance in [Camera::MIN_DISTANCE, Camera::DEFAULT_DISTANCE, Camera::MAX_DISTANCE] {
            {
                let projection = Projection::Cabinet;
                let camera = Camera { projection, distance, ..Camera::default() };
                let drawn = view.scrolled(&camera, aspect);
                // The widest lattice step, on either axis, that the pane shows.
                let mut shown = LatticePos::new(0, 0, 0);
                for pos in drawn.visible_positions() {
                    let Some(p) = ndc(&camera, &drawn, aspect, pos) else {
                        continue;
                    };
                    if p.x.abs() <= 1.0 && p.y.abs() <= 1.0 {
                        shown.threes = shown.threes.max((pos.threes - drawn.center_threes).abs());
                        shown.fives = shown.fives.max((pos.fives - drawn.center_fives).abs());
                    }
                }
                for (drawn_extent, shown_extent, axis) in [
                    (drawn.extent_threes, shown.threes, "fifths"),
                    (drawn.extent_fives, shown.fives, "thirds"),
                ] {
                    assert!(
                        drawn_extent <= shown_extent + 4,
                        "{projection:?} at aspect {aspect}, distance {distance}: the {axis} \
                         window reaches {drawn_extent} steps out where the pane shows only \
                         {shown_extent}",
                    );
                }
            }
        }
    }
}

/// Scrolling does not run out of lattice: pan a long way and the pane is still
/// full, under EVERY projection.
///
/// The whole feature in one test, and swept over all three deliberately.
/// Cabinet is the projection that cannot fail it — its pan runs along the
/// sheet by construction — so a test that panned only the default camera was
/// checking the one case that was already right. The other two move the
/// target through the sheets as well as across them, and left unhandled that
/// walked the lattice clean off the pane while the frame went on deriving
/// twenty thousand nodes for it.
#[test]
fn panning_a_long_way_keeps_the_window_full() {
    let aspect = 16.0 / 9.0;
    for projection in PROJECTIONS {
        let mut view = ViewConfig::default();
        let mut camera = Camera { projection, ..Camera::default() };
        let at_home = view.scrolled(&camera, aspect);

        // Two hundred steps out along both axes, in the drags a hand would
        // make — and the drags go through `pan`, so whatever a real gesture
        // does to the target is what this walks into.
        for _ in 0..400 {
            camera.pan(Vec2::new(-120.0, -60.0));
            view.follow_camera(&mut camera);
        }
        let far = view.scrolled(&camera, aspect);
        // Within a step of the size it started at, rather than exactly it:
        // the follow leaves the target inside its cell but not ON the origin,
        // and the window is sized from the origin, so a half-cell residual is
        // worth one more step on the far side. What this is guarding is that
        // the size does not GROW with the distance scrolled.
        for (far_extent, home_extent, axis) in [
            (far.extent_threes, at_home.extent_threes, "fifths"),
            (far.extent_fives, at_home.extent_fives, "thirds"),
        ] {
            assert!(
                (far_extent - home_extent).abs() <= 1,
                "{projection:?}: the {axis} window went from {home_extent} to {far_extent} on \
                 the way out",
            );
        }
        assert!(
            far.center_fives.abs() > 20 || far.center_threes.abs() > 20,
            "{projection:?}: the pan did not move the window far enough to be a test: {:?}",
            far.center(),
        );
        // Nodes are still ON the pane, which is the claim the count above
        // cannot make: a window of the right size, drawn where nobody is
        // looking, is the failure this is here to catch.
        let on_pane = far
            .visible_positions()
            .filter(|&pos| {
                ndc(&camera, &far, aspect, pos)
                    .is_some_and(|p| p.x.abs() <= 1.0 && p.y.abs() <= 1.0)
            })
            .count();
        assert!(on_pane > 100, "{projection:?}: only {on_pane} nodes are on the pane after panning");

        // The target is left inside one cell of the origin on EVERY axis. The
        // depth is the one that would otherwise grow without bound, since a
        // tilted camera's pan carries one — and the target is persisted.
        assert!(
            camera.target.abs().max_element() <= view.spacing,
            "{projection:?}: the target walked off with the window: {:?}",
            camera.target,
        );
        // The reach the names are chosen out of came along, so the notes on
        // screen are still ones the analyzer can name.
        //
        // Held strictly for cabinet alone, which is what the reach is sized
        // on. A tilted camera's window has to reach back to the world origin
        // as well as across the pane (see `scrolled`), so its far corner can
        // sit outside a reach that covers everything actually visible — and
        // a reach sized for THAT is a walk of thousands per played pitch,
        // every frame, to name the corner of a picture nobody is reading.
        let reach: std::collections::HashSet<_> = view.visible_positions().collect();
        let outside = far
            .visible_positions()
            .filter(|&pos| {
                ndc(&camera, &far, aspect, pos)
                    .is_some_and(|p| p.x.abs() <= 1.0 && p.y.abs() <= 1.0)
            })
            .filter(|pos| !reach.contains(pos))
            .count();
        if projection == Projection::Cabinet {
            assert_eq!(outside, 0, "cabinet: {outside} nodes on screen are outside the reach");
        }
    }
}

/// Following the camera moves the window and the camera together, so the
/// picture does not stir: a whole step added to the center subtracts one
/// spacing from every node's world position, and the target has to lose the
/// same or the lattice jumps a node's width as you drag past a cell boundary.
#[test]
fn following_the_camera_does_not_move_the_picture() {
    let aspect = 1.5;
    // Most of a cell, so the follow really does take a step — the boundary is
    // where a bug here would show, as the lattice jumping a node's width
    // mid-drag.
    let moved = Vec3::new(0.8, -0.7, 0.0);

    // The same camera move with and without the follow. Stated as two runs
    // rather than as an expected screen position, because the position the
    // camera's own move accounts for is exactly what the untouched run is —
    // and deriving it by hand would only be a second chance to get the
    // projection wrong.
    let plain_view = ViewConfig::default();
    let mut plain_camera = Camera::default();
    plain_camera.target += moved;

    let (mut view, mut camera) = (ViewConfig::default(), Camera::default());
    camera.target += moved;
    view.follow_camera(&mut camera);
    assert_eq!(
        (view.center_fives, view.center_threes),
        (1, -1),
        "the window did not take the step the camera did",
    );

    for pos in coords::positions_within(-4..=4, -4..=4, 0..=0) {
        let (Some(followed), Some(plain)) = (
            ndc(&camera, &view, aspect, pos),
            ndc(&plain_camera, &plain_view, aspect, pos),
        ) else {
            continue;
        };
        assert!(
            (followed - plain).abs().max_element() < 1e-5,
            "{pos:?} draws at {followed:?} once the window has followed and {plain:?} \
             before, so the follow moved the picture",
        );
    }
}

/// The zoom limit is what bounds the picture: fully zoomed out the pane holds
/// a readable number of nodes rather than a field of specks.
///
/// The number is the shape of the thing, not a value to defend — twenty steps
/// of fifths tall, and whatever the pane's own aspect makes of that across.
#[test]
fn the_zoom_limit_lands_near_twenty_steps() {
    let view = ViewConfig::default();
    let camera = Camera { distance: Camera::MAX_DISTANCE, ..Camera::default() };
    let square = view.scrolled(&camera, 1.0);
    let tall = 2 * square.extent_threes + 1;
    assert!(
        (18..=26).contains(&tall),
        "fully zoomed out the pane is {tall} steps of fifths tall",
    );
    // A wider pane gets its extra width, and nothing else: the height is a
    // property of the zoom, the width of the pane.
    let wide = view.scrolled(&camera, 16.0 / 9.0);
    assert_eq!(wide.extent_threes, square.extent_threes);
    assert!(wide.extent_fives > square.extent_fives);
}

/// No camera at all draws more than [`MAX_DRAWN_NODES`] — including the ones
/// that have no sensible answer.
///
/// A steep perspective pitch is the case worth naming: the sheet goes edge-on,
/// so the span it shows really is unbounded, and the honest window is infinite.
/// The picture there is a line, and what it must not be is a stall.
#[test]
fn no_camera_asks_for_more_than_the_budget() {
    let view = ViewConfig { extent_sevens: 4, ..ViewConfig::default() };
    for projection in PROJECTIONS {
        for aspect in [0.05, 1.0, 20.0] {
            for pitch in [0.0, 1.0, Camera::PITCH_LIMIT] {
                for distance in [Camera::MIN_DISTANCE, Camera::MAX_DISTANCE] {
                    let camera = Camera { projection, pitch, distance, ..Camera::default() };
                    let count = view.scrolled(&camera, aspect).visible_count();
                    assert!(
                        count <= MAX_DRAWN_NODES,
                        "{projection:?} at aspect {aspect}, pitch {pitch}, distance {distance} \
                         asked for {count} nodes",
                    );
                }
            }
        }
    }
}

/// A target no gesture can reach still leaves both windows usable, and walks
/// back to somewhere the picture is.
///
/// `Camera::sanitize` deliberately leaves the target unbounded ("No bound the
/// geometry cares about"), so this is a value that really does arrive from a
/// blob — and `follow_camera` is the function on the far side of that door
/// that WRITES persisted state from it. Carrying the step into the center
/// without bounding it pinned the center at `i32::MAX` while the target went
/// on stepping, and `center + extent` in `visible_positions` then wrapped:
/// the reach came out EMPTY, so every note read as off the lattice, with no
/// analyzer names and an empty Notes column, and it never recovered.
#[test]
fn a_target_no_gesture_can_reach_still_names_and_draws() {
    for x in [1e9f32, 3e9, 1e30, -1e30, f32::MAX] {
        let mut view = ViewConfig::default();
        let mut camera = Camera { target: Vec3::new(x, -x, x), ..Camera::default() };
        // A few frames, because an absurd target is walked back rather than
        // teleported — one bound's worth of steps per frame.
        for _ in 0..8 {
            view.follow_camera(&mut camera);
        }
        assert!(
            view.visible_positions().count() > 0,
            "target {x:e} left the naming reach empty at center {:?}",
            view.center(),
        );
        // The sum `visible_positions` builds its ranges from, which is what
        // wrapped. In debug this test would panic on the overflow instead.
        assert!(
            view.center_fives.checked_add(view.extent_fives).is_some()
                && view.center_threes.checked_add(view.extent_threes).is_some(),
            "target {x:e} left a center that overflows its own extent: {:?}",
            view.center(),
        );
        assert!(view.scrolled(&camera, 1.5).visible_count() > 0);
    }
}

/// A camera a hand-edited blob can carry still yields a drawable window. The
/// same door `Camera::sanitize` and `ViewConfig::sanitize` guard, one step
/// further along: these two run on load, and a scene is derived every frame
/// from whatever they left.
#[test]
fn a_nonsense_camera_still_yields_a_drawable_window() {
    for spacing in [1.0, 0.0, -1.0, f32::NAN] {
        let view = ViewConfig { spacing, ..ViewConfig::default() };
        for target in [Vec3::ZERO, Vec3::splat(1e9), Vec3::splat(f32::NAN)] {
            for aspect in [1.5, 0.0, f32::NAN] {
                let camera = Camera { target, ..Camera::default() };
                let drawn = view.scrolled(&camera, aspect);
                assert!(
                    drawn.extent_threes >= 0 && drawn.extent_fives >= 0,
                    "spacing {spacing}, target {target:?}, aspect {aspect} gave a negative \
                     window: {}x{}",
                    drawn.extent_threes,
                    drawn.extent_fives,
                );
                assert!(drawn.visible_count() <= MAX_DRAWN_NODES);
            }
        }
    }
}
