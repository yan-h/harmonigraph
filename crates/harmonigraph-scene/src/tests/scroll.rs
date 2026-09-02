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
            for distance in [Camera::MIN_DISTANCE, Camera::DEFAULT_DISTANCE, Camera::MAX_DISTANCE] {
                let camera = Camera { projection, distance, ..Camera::default() };
                let drawn = view.scrolled(&camera, aspect);
                if projection != Projection::Cabinet && drawn.count() > MAX_DRAWN_NODES - 512 {
                    continue;
                }
                let drawn_set: std::collections::HashSet<_> = drawn.positions().collect();
                // Wider than any window this camera can produce, so the search
                // is not the window's own claim about itself.
                for pos in coords::positions_within(-90..=90, -90..=90, -2..=2) {
                    let Some(p) = ndc(&camera, &view, aspect, pos) else {
                        continue;
                    };
                    if p.x.abs() > 1.0 || p.y.abs() > 1.0 {
                        continue;
                    }
                    assert!(
                        drawn_set.contains(&pos),
                        "{projection:?} at aspect {aspect}, distance {distance}: {pos:?} lands \
                         on the pane at {p:?} and is not in the drawn window \
                         ({:?}..{:?})",
                        drawn.min,
                        drawn.max,
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
    let reach: std::collections::HashSet<_> = view.reach().positions().collect();
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
                    .positions()
                    .filter(|&pos| {
                        ndc(&camera, &view, aspect, pos)
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

/// ...and a LOADED view holds it too, which is the half the fresh one cannot
/// check.
///
/// The reach is sized on the drawn window, and the sizing is a default — so the
/// test above asserts the property for the one view that cannot violate it. A
/// saved blob carries whatever reach the build that wrote it shipped, and the
/// reach is a persisted field with no bar: nothing on screen sets it, so
/// nothing on screen can repair it either. Sanitize is the only place a loaded
/// view is made to make sense, and a reach that no longer covers the window is
/// exactly the case it exists for.
///
/// The fixture is the reach the previous build shipped, which is the input
/// every project on disk actually carries.
#[test]
fn a_loaded_view_never_draws_a_node_its_reach_cannot_name() {
    let mut view = ViewConfig { extent_threes: 10, extent_fives: 6, ..ViewConfig::default() };
    view.sanitize();
    let reach: std::collections::HashSet<_> = view.reach().positions().collect();
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
                    .positions()
                    .filter(|&pos| {
                        ndc(&camera, &view, aspect, pos)
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

/// No cabinet camera reaches the node budget on a pane of ordinary shape, at
/// any zoom, depth or shear — so under the projection this feature is FOR, the
/// cap is a thing that exists rather than a thing that happens.
///
/// Ordinary is the load-bearing word, and 3:1 is where the sweep stops because
/// that is about where the guarantee does. Cabinet's window is bounded at
/// every setting — it faces the sheet, so no orbit lays the lattice edge-on —
/// but bounded is not the same as under the cap: past roughly 3.5:1 with full
/// depth it reaches this one, and `Layout::split` will hand the lattice a
/// fifth of a 21:9 frame, which is 11.7:1. See [`MAX_DRAWN_NODES`] for what
/// the trim costs there and why the cap is not simply raised past it.
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
                    let count = view.scrolled(&camera, aspect).count();
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
/// Each SIDE on its own, which is the half a symmetric window could not be
/// held to. A `center ± extent` window has to cover the farther side and then
/// mirrors that reach onto the nearer one, so under any camera whose view of
/// the sheet is lopsided it draws a second copy of its own far reach where the
/// pane shows nothing — and the only way to state a bound it could pass was
/// per-axis, cabinet-only, which is exactly the camera that is symmetric
/// anyway. Bounds of their own let all three projections be held to what the
/// pane actually shows, on all four sides.
#[test]
fn the_window_is_not_much_wider_than_the_pane() {
    let view = ViewConfig::default();
    let center = view.center();
    for projection in PROJECTIONS {
        for aspect in ASPECTS {
            for distance in [Camera::MIN_DISTANCE, Camera::DEFAULT_DISTANCE, Camera::MAX_DISTANCE] {
                let camera = Camera { projection, distance, ..Camera::default() };
                let drawn = view.scrolled(&camera, aspect);
                // The farthest step, on each side of each axis, the pane shows.
                let (mut lo, mut hi) = (center, center);
                for pos in drawn.positions() {
                    let Some(p) = ndc(&camera, &view, aspect, pos) else {
                        continue;
                    };
                    if p.x.abs() <= 1.0 && p.y.abs() <= 1.0 {
                        lo = LatticePos::new(
                            lo.threes.min(pos.threes),
                            lo.fives.min(pos.fives),
                            lo.sevens,
                        );
                        hi = LatticePos::new(
                            hi.threes.max(pos.threes),
                            hi.fives.max(pos.fives),
                            hi.sevens,
                        );
                    }
                }
                for (drawn_edge, shown_edge, side) in [
                    (drawn.min.threes, lo.threes, "bottom of the thirds"),
                    (drawn.max.threes, hi.threes, "top of the thirds"),
                    (drawn.min.fives, lo.fives, "low end of the fifths"),
                    (drawn.max.fives, hi.fives, "high end of the fifths"),
                ] {
                    assert!(
                        (drawn_edge - shown_edge).abs() <= 4,
                        "{projection:?} at aspect {aspect}, distance {distance}: the \
                         {side} window reaches {drawn_edge} where the pane shows only \
                         {shown_edge}",
                    );
                }
            }
        }
    }
}

/// The window is LOPSIDED where the camera's view of the sheet is, which is
/// the whole of what keeps the far field out of the frame.
///
/// Perspective is where this bites: tilt the eye and the sheet runs away from
/// it on one side and off the bottom of the pane on the other, so the honest
/// window has one long side and one short one. Sized as `center ± extent` it
/// has to take the long side twice — the pane never shows the second copy, and
/// at 40° on a 16:9 pane that is 25921 nodes asked for against 9494 the camera
/// can see. The cap then rations a budget inflated 2.7x, and starts trimming
/// nodes the pane really does show.
///
/// Stated as the ratio against the mirrored window rather than as a node
/// count, because the count is the camera's business and the doubling is this
/// function's.
///
/// Held over the tilts where the pane's view of the sheet is BOUNDED, which is
/// where a lopsided window is a thing that exists. Past about 45° a corner of
/// the viewport clears the horizon and what the pane shows has no far edge at
/// all — see [`Camera::visible_world_bounds`] — and there `scrolled` mirrors
/// on purpose, which the second half of this pins.
#[test]
fn a_tilted_window_does_not_mirror_its_far_side() {
    let view = ViewConfig::default();
    let center = view.center();
    // What `center ± extent` would have forced: each axis taken to its
    // farther end and mirrored.
    let mirror = |lo: i32, hi: i32, c: i32| 2 * (c - lo).max(hi - c) + 1;
    let tilted = |pitch_deg: f32| {
        let camera = Camera {
            projection: Projection::Perspective,
            pitch: pitch_deg.to_radians(),
            distance: Camera::MAX_DISTANCE,
            ..Camera::default()
        };
        view.scrolled(&camera, 16.0 / 9.0)
    };
    let mut worst: f64 = 1.0;
    for pitch_deg in [20.0f32, 40.0] {
        let drawn = tilted(pitch_deg);
        let mirrored = mirror(drawn.min.threes, drawn.max.threes, center.threes) as f64
            * mirror(drawn.min.fives, drawn.max.fives, center.fives) as f64;
        let ratio = mirrored / drawn.count() as f64;
        assert!(
            ratio > 1.5,
            "at pitch {pitch_deg}° the window is {} nodes against {mirrored} mirrored \
             ({ratio:.2}x) — it is not tracking the lopsided view at all",
            drawn.count(),
        );
        worst = worst.max(ratio);
    }
    assert!(worst > 2.5, "the worst mirroring saved is only {worst:.2}x");

    // Past the horizon the mirror is the ANSWER rather than the bug: there is
    // no far edge to track, and a window that took the rectangle's two sides
    // for edges would start a step from the center with the foreground gone.
    let over = tilted(60.0);
    assert_eq!(
        (over.max.threes - center.threes, over.max.fives - center.fives),
        (center.threes - over.min.threes, center.fives - over.min.fives),
        "a view with no far edge was still drawn as if it had one: {:?}..{:?}",
        over.min,
        over.max,
    );
}

/// What the node budget trims is the HORIZON, never the foreground.
///
/// The sibling above holds that a tilted window is lopsided; this holds that
/// the cap keeps it that way. A lopsided block has a step or two of near sheet
/// on one side and hundreds of sub-pixel far field on the other, so a trim
/// that scales both bounds by one factor takes the same FRACTION off each —
/// which off a near edge already beside the center is the whole of it. The
/// picture that draws is a bald wedge across the lower half of the pane with
/// the lattice running on to the horizon above it: at 52° on a 16:9 pane the
/// window came out `threes -1..161`, so every node below the center was gone
/// while 161 steps of far field survived.
///
/// Swept over the tilt, which is the axis the rest of this file pins at
/// `Camera::default`'s 0.3 — and 0.3 is flat enough that no perspective camera
/// there reaches the cap at all, so the trim these tests exercise is the one
/// that never runs.
#[test]
fn the_budget_trims_the_horizon_not_the_foreground() {
    // Near enough the center to be the picture rather than the far field: at
    // the zoom limit the pane is about twenty steps tall all told.
    const FOREGROUND: i32 = 12;
    for sevens in [0, 2, 4] {
        let view = ViewConfig { extent_sevens: sevens, ..ViewConfig::default() };
        let center = view.center();
        for pitch in [0.6f32, 0.75, 0.9] {
            for distance in [Camera::DEFAULT_DISTANCE, Camera::MAX_DISTANCE] {
                for aspect in ASPECTS {
                    let camera = Camera {
                        projection: Projection::Perspective,
                        pitch,
                        distance,
                        ..Camera::default()
                    };
                    let drawn = view.scrolled(&camera, aspect);
                    for pos in coords::positions_within(
                        center.threes - FOREGROUND..=center.threes + FOREGROUND,
                        center.fives - FOREGROUND..=center.fives + FOREGROUND,
                        center.sevens..=center.sevens,
                    ) {
                        let Some(p) = ndc(&camera, &view, aspect, pos) else {
                            continue;
                        };
                        if p.x.abs() > 1.0 || p.y.abs() > 1.0 {
                            continue;
                        }
                        assert!(
                            drawn.contains(pos),
                            "{sevens} sheets, pitch {pitch}, distance {distance}, aspect \
                             {aspect}: {pos:?} lands on the pane at {p:?} and the budget \
                             trimmed it away ({:?}..{:?}, {} nodes)",
                            drawn.min,
                            drawn.max,
                            drawn.count(),
                        );
                    }
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
/// sheet by construction — so panning only the default camera would check the
/// one case that is right for free. The other two move the target through the
/// sheets as well as across them, which unhandled walks the lattice clean off
/// the pane while the frame goes on deriving twenty thousand nodes for it.
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
        let span = |w: &crate::DrawnWindow, axis: fn(LatticePos) -> i32| -> i32 {
            axis(w.max) - axis(w.min)
        };
        for (far_span, home_span, axis) in [
            (span(&far, |p| p.threes), span(&at_home, |p| p.threes), "thirds"),
            (span(&far, |p| p.fives), span(&at_home, |p| p.fives), "fifths"),
        ] {
            assert!(
                (far_span - home_span).abs() <= 1,
                "{projection:?}: the {axis} window went from {home_span} to {far_span} on \
                 the way out",
            );
        }
        assert!(
            view.center_fives.abs() > 20 || view.center_threes.abs() > 20,
            "{projection:?}: the pan did not move the window far enough to be a test: {:?}",
            view.center(),
        );
        // Nodes are still ON the pane, which is the claim the count above
        // cannot make: a window of the right size, drawn where nobody is
        // looking, is the failure this is here to catch.
        let on_pane = far
            .positions()
            .filter(|&pos| {
                ndc(&camera, &view, aspect, pos)
                    .is_some_and(|p| p.x.abs() <= 1.0 && p.y.abs() <= 1.0)
            })
            .count();
        assert!(
            on_pane > 100,
            "{projection:?}: only {on_pane} nodes are on the pane after panning",
        );

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
        let reach: std::collections::HashSet<_> = view.reach().positions().collect();
        let outside = far
            .positions()
            .filter(|&pos| {
                ndc(&camera, &view, aspect, pos)
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
    let before = (view.center_fives, view.center_threes);
    camera.target += moved;
    view.follow_camera(&mut camera);
    assert_eq!(
        (view.center_fives, view.center_threes),
        (before.0 + 1, before.1 - 1),
        "the window did not take the step the camera did",
    );

    for pos in coords::positions_within(-4..=4, -4..=4, 0..=0) {
        let (Some(followed), Some(plain)) =
            (ndc(&camera, &view, aspect, pos), ndc(&plain_camera, &plain_view, aspect, pos))
        else {
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
    let tall = square.max.threes - square.min.threes + 1;
    assert!((18..=26).contains(&tall), "fully zoomed out the pane is {tall} steps of thirds tall",);
    // A wider pane gets its extra width, and nothing else: the height is a
    // property of the zoom, the width of the pane.
    let wide = view.scrolled(&camera, 16.0 / 9.0);
    assert_eq!(wide.max.threes - wide.min.threes, square.max.threes - square.min.threes);
    assert!(wide.max.fives - wide.min.fives > square.max.fives - square.min.fives);
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
                    let count = view.scrolled(&camera, aspect).count();
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
/// on stepping, and `center + extent` in `reach` then wrapped:
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
            view.reach().positions().count() > 0,
            "target {x:e} left the naming reach empty at center {:?}",
            view.center(),
        );
        // The sum `reach` builds its ranges from, which is what wrapped. In
        // debug this test would panic on the overflow instead.
        assert!(
            view.center_fives.checked_add(view.extent_fives).is_some()
                && view.center_threes.checked_add(view.extent_threes).is_some(),
            "target {x:e} left a center that overflows its own extent: {:?}",
            view.center(),
        );
        assert!(view.scrolled(&camera, 1.5).count() > 0);
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
                    drawn.min.threes <= drawn.max.threes && drawn.min.fives <= drawn.max.fives,
                    "spacing {spacing}, target {target:?}, aspect {aspect} gave an inverted \
                     window: {:?}..{:?}",
                    drawn.min,
                    drawn.max,
                );
                assert!(drawn.count() <= MAX_DRAWN_NODES);
            }
        }
    }
}

/// `index_of` inverts `positions` exactly, on a block that is lopsided on
/// every axis.
///
/// The window finds a node by arithmetic rather than by hashing, which is only
/// sound while the walk and the inverse agree — and the iteration order is
/// load-bearing here rather than incidental. A symmetric block hides a whole
/// class of mistake, because a min derived as `-extent` and a span derived as
/// `2 * extent + 1` agree with each other even when both are wrong about
/// where the block starts.
#[test]
fn the_index_of_a_position_is_where_the_walk_puts_it() {
    let window = DrawnWindow { min: LatticePos::new(-7, -2, -1), max: LatticePos::new(3, 9, 2) };
    let walked: Vec<_> = window.positions().collect();
    assert_eq!(walked.len(), window.count(), "the count does not match the walk");
    for (i, pos) in walked.iter().enumerate() {
        assert_eq!(window.index_of(*pos), Some(i), "{pos:?} is not at its own index");
    }
    // And nothing outside it lands on a slot at all — one step past each face,
    // which is exactly the step a caller takes looking for a neighbour.
    for outside in [
        LatticePos::new(window.min.threes - 1, 0, 0),
        LatticePos::new(window.max.threes + 1, 0, 0),
        LatticePos::new(0, window.min.fives - 1, 0),
        LatticePos::new(0, window.max.fives + 1, 0),
        LatticePos::new(0, 0, window.min.sevens - 1),
        LatticePos::new(0, 0, window.max.sevens + 1),
    ] {
        assert_eq!(window.index_of(outside), None, "{outside:?} is outside and was given a slot");
    }
}
