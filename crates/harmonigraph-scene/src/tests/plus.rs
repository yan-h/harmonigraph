//! The lattice structure standing AT the nodes: which positions carry a marker,
//! how big it is, and the one grey it shares with everything else at rest.
//!
//! Nothing is drawn BETWEEN the positions, and several tests here exist to
//! hold that: the field's regularity is what the rows and columns are read
//! off, so the resting picture is a set of marks and not a mesh.

use crate::*;
use harmonigraph_core::{LatticePos, NoteEvent, NoteTracker, Tuning};
use super::harness::*;

/// A 7x7 window one sevens step deep, so off-sheet positions are in the mix
/// and can be shown to carry no marker.
///
/// The Show row is PINNED to the one mode that names nothing at rest, and it
/// has to be: a named position draws no marker, so every geometry and colour
/// claim below would otherwise be measuring the Show row rather than its own
/// subject — and under `All` there would be no marker left to measure at all. The
/// naming rule has its own tests, which set the mode themselves.
fn plus_view() -> ViewConfig {
    ViewConfig {
        extent_threes: 3,
        extent_fives: 3,
        extent_sevens: 1,
        note_names: NoteNames::Played,
        ..plain_view()
    }
}

/// A note played and let go of long enough ago that its fade is over and it
/// has moved out of the live voices into history — which is what
/// `TrailField::build` reads, and so the only way to get a REMEMBERED position
/// rather than a releasing one.
fn played_and_forgotten() -> NoteTracker {
    let mut tracker = NoteTracker::new();
    for (time, kind) in [
        (0.0, harmonigraph_core::NoteEventKind::On { velocity: 1.0 }),
        (1.0, harmonigraph_core::NoteEventKind::Off),
    ] {
        tracker.handle_event(NoteEvent { time, channel: 0, note: 60, kind });
    }
    tracker.prune(3.0, &harmonigraph_core::Envelope::default());
    tracker
}

fn pluses_of(view: &ViewConfig) -> Vec<PlusInstance> {
    pluses_of_with(view, &NoteTracker::new())
}

fn pluses_of_with(view: &ViewConfig, tracker: &NoteTracker) -> Vec<PlusInstance> {
    scene_of(tracker, &Tuning::default(), view, &plain_frame(), 0.0).pluses
}

fn span_of(view: &ViewConfig) -> f32 {
    scene_of(&NoteTracker::new(), &Tuning::default(), view, &plain_frame(), 0.0).marker_span
}

fn unit_of(view: &ViewConfig) -> f32 {
    scene_of(&NoteTracker::new(), &Tuning::default(), view, &plain_frame(), 0.0).marker_unit
}

/// `marker_unit` converts the marker field between the world its draws are in
/// and the quad uv its bars are dialled in — an arm and a pool both read back as
/// the number the bar behind them holds.
///
/// The shader's own reason for wanting it: the markers' light draw is handed
/// world lengths (a billboard, and an arm per instance) and holds that light off
/// by the Gap, which is a node's bar and so in uv. One of the two has to be
/// converted, and the conversion is the SCENE's rather than any marker's —
/// carrying it over is what keeps `lattice.wgsl` from spelling the uv rule a
/// second time for one more layer.
///
/// Read against the bars rather than against a formula, which is the whole
/// point: a unit that disagreed with them would leave the marker's shadow drawn
/// at a different Gap than every node's, on one bar.
#[test]
fn the_marker_unit_is_what_reads_a_markers_world_back_as_its_bars() {
    let view = ViewConfig { plus_arm: 0.2, marker_reach: 0.3, ..plus_view() };
    let unit = unit_of(&view);
    assert!(unit > 0.0, "the field must have a unit to be measured in");
    let arm = pluses_of(&view)[0].radius;
    assert!(
        (arm / unit - 0.2).abs() < 1e-4,
        "an arm of 0.2 read back as {}",
        arm / unit,
    );
    assert!(
        (span_of(&view) / unit - 0.5).abs() < 1e-4,
        "an arm of 0.2 and a reach of 0.3 spanned {}",
        span_of(&view) / unit,
    );
    // The spacing is the one view bar under it, and it scales the field whole:
    // both lengths move with the unit, so the bars read the same at either.
    let wide = ViewConfig { spacing: view.spacing * 3.0, ..view.clone() };
    assert!(
        (unit_of(&wide) - unit * 3.0).abs() < 1e-4,
        "trebling the spacing moved the unit to {}",
        unit_of(&wide),
    );
    assert!(
        (pluses_of(&wide)[0].radius / unit_of(&wide) - 0.2).abs() < 1e-4,
        "a wider lattice read its own arm back as {}",
        pluses_of(&wide)[0].radius / unit_of(&wide),
    );
}

/// A marker's pool spans its own ARM plus the Marker reach, and takes nothing
/// from the node Reach beside it.
///
/// The shape is the node's own one rung down — a node's light spans its
/// outermost drawn edge plus the Reach — so a reach of 0 is a pool the size of
/// the cross rather than no pool, and what turns the light off is its level.
///
/// The independence is the half worth pinning. The two distances are spent on
/// wildly different numbers of lights: a few nodes sound at once, and every
/// lattice position carries a marker that lights always, so the reach that
/// turns a node's halo into a field turns the marker field into one flat wash.
/// Sharing one bar between them is the change this test exists to fail.
#[test]
fn a_markers_pool_spans_its_arm_plus_its_own_reach() {
    let at = |arm: f32, marker: f32, glow: f32| -> f32 {
        span_of(&ViewConfig {
            plus_arm: arm,
            marker_reach: marker,
            glow_reach: glow,
            ..plus_view()
        })
    };
    // One arm's world length, read off the marker itself rather than converted
    // here: the same number the instance carries, so the sums below are in the
    // units the pool is actually drawn in.
    let arm = pluses_of(&ViewConfig { plus_arm: 0.2, ..plus_view() })[0].radius;
    assert!(arm > 0.0, "the fixture must draw a marker to measure an arm by");
    assert!(
        (at(0.2, 0.0, 1.0) - arm).abs() < 1e-4,
        "a reach of 0 must span the arm exactly: {} against {arm}",
        at(0.2, 0.0, 1.0),
    );
    assert!(
        (at(0.2, 0.2, 1.0) - arm * 2.0).abs() < 1e-4,
        "an arm's worth of reach must double the span: {} against {}",
        at(0.2, 0.2, 1.0),
        arm * 2.0,
    );
    for glow in [0.0, GLOW_REACH_MAX] {
        assert!(
            (at(0.2, 0.3, glow) - at(0.2, 0.3, 1.0)).abs() < 1e-6,
            "the node Reach at {glow} moved a marker's pool",
        );
    }
    assert!(
        (at(0.2, MARKER_REACH_MAX * 2.0, 1.0) - at(0.2, MARKER_REACH_MAX, 1.0)).abs() < 1e-6,
        "a reach past the ceiling must clamp to it",
    );
}

#[test]
fn a_marker_stands_at_every_home_position_and_nowhere_else() {
    // The marker field IS the lattice's resting picture, so "one per home
    // position" is the whole shape of it: a position with no marker is a
    // position nobody can see, and a marker with no position is a mark from
    // nowhere. Both halves, because a count alone passes on a field that has
    // drifted off the nodes entirely.
    let view = plus_view();
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
    let home: Vec<&NodeInstance> = scene.nodes.iter().filter(|n| n.on_home).collect();
    assert!(!home.is_empty() && home.len() < scene.nodes.len(), "want both kinds in the window");
    assert_eq!(scene.pluses.len(), home.len(), "one marker per home position, and no others");
    for node in &home {
        assert!(
            scene.pluses.iter().any(|d| d.pos == node.world_pos),
            "no marker standing at the home position {:?}",
            node.lattice_pos,
        );
    }
    for node in scene.nodes.iter().filter(|n| !n.on_home) {
        assert!(
            !scene.pluses.iter().any(|d| d.pos == node.world_pos),
            "an off-sheet position at {:?} was marked; the home sheet is the ground \
             because it is the only one that draws at rest",
            node.lattice_pos,
        );
    }

    // Panning the window keeps the markers attached to the visible nodes (both
    // are derived in centered world space).
    let panned = ViewConfig { center_threes: 3, ..view };
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &panned, &plain_frame(), 0.0);
    for marker in &scene.pluses {
        assert!(
            scene.nodes.iter().any(|n| n.world_pos == marker.pos),
            "a panned marker at {:?} stands at no node",
            marker.pos,
        );
    }
}

#[test]
fn nothing_is_drawn_between_two_positions() {
    // The claim the lines used to make and no longer do. A marker stands ON a
    // node, so every one of them is at a position the window holds — and a
    // mark at the midpoint of two of them would be an interval drawn, which
    // is exactly what a mesh says and a field does not.
    let scene =
        scene_of(&NoteTracker::new(), &Tuning::default(), &plus_view(), &plain_frame(), 0.0);
    for marker in &scene.pluses {
        assert!(
            scene.nodes.iter().any(|n| n.world_pos == marker.pos),
            "a marker at {:?} stands between positions rather than at one",
            marker.pos,
        );
    }
}

#[test]
fn the_arm_bar_sets_how_far_a_marker_reaches_and_0_takes_it_away() {
    // The size bar is the markers' own off switch, and skipping the instances is
    // the same picture the shader would discard to — so 0 has to ship an
    // empty field rather than a field of nothing.
    assert!(
        pluses_of(&ViewConfig { plus_arm: 0.0, ..plus_view() }).is_empty(),
        "at 0 the pluses are off, so no instance should be shipped",
    );

    let mut radii = vec![];
    for size in [0.05f32, 0.2, 0.5, PLUS_SIZE_MAX] {
        let pluses = pluses_of(&ViewConfig { plus_arm: size, ..plus_view() });
        let radius = pluses[0].radius;
        assert!(
            pluses.iter().all(|d| d.radius == radius),
            "every marker in one field is the same size",
        );
        radii.push(radius);
    }
    for pair in radii.windows(2) {
        assert!(pair[1] > pair[0], "more size must mean a bigger marker: {radii:?}");
    }

    // The bar reads in the quad UV a node's ring radii are dialled in, and
    // what reaches the scene is a WORLD length — the conversion is spent once,
    // here, so nothing downstream carries a second copy of the convention.
    let view = ViewConfig { plus_arm: 0.5, ..plus_view() };
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
    let want = scene.node_radius * 1.8 * 0.5;
    assert!(
        (scene.pluses[0].radius - want).abs() < 1e-5,
        "a uv of 0.5 is {want} in world units, got {}",
        scene.pluses[0].radius,
    );
}

/// The view keeps the width as a LENGTH beside the arm, because that is what
/// makes the two bars independent; the shader needs HALF of it as a share of
/// the arm, its uv being the arm's own units. This pins that conversion —
/// including the square at the top of the bar, and the ends where dividing is
/// the obvious way to get it wrong.
#[test]
fn the_width_reaches_the_scene_as_a_share_of_the_arm() {
    // (arm, width) -> half the thickness, as a share of one arm.
    for (arm, width, want) in [
        // The fresh proportion: a little over half an arm across.
        (0.2f32, 0.11f32, 0.275f32),
        // Half of it, taken from either end of the bar — the shader measures
        // out from the arm's own centre line, so the whole thickness is never
        // what reaches it.
        (0.4, 0.2, 0.25),
        (0.2, 0.1, 0.25),
        // Twice the arm across, and the cross has filled its own square. Past
        // that it stays one rather than running off the end of the octant.
        (0.2, 0.4, 1.0),
        (0.2, 0.9, 1.0),
        // A width below nothing is the thinnest cross rather than a shape
        // turned inside out.
        (0.2, -1.0, 0.0),
        // No arm at all: `derive_pluses` draws nothing here, so what matters is
        // that asking costs no division by zero.
        (0.0, 0.5, 0.0),
    ] {
        let view = ViewConfig { plus_arm: arm, plus_width: width, ..plus_view() };
        let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
        assert!(
            (scene.plus_half_width - want).abs() < 1e-5,
            "a {width} width on a {arm} arm reaches the scene as {}, wanted {want}",
            scene.plus_half_width,
        );
    }
}

/// The view keeps the taper as a WIDTH beside the reach, because that is the
/// pair the two-handle bar sets; the shader needs the POINT on an axis whose 1
/// is the arm's tip. This pins that conversion, including the two ends where
/// dividing is the obvious way to get it wrong.
#[test]
fn the_taper_reaches_the_scene_as_a_share_of_the_arm() {
    // (size, taper) -> where the fade starts, as a share of one arm.
    for (size, taper, want) in [
        // Half the arm tapered, from either side of the bar.
        (0.4f32, 0.2f32, 0.5f32),
        (0.2, 0.1, 0.5),
        // A square end. Held a thousandth short of the tip rather than AT it,
        // so the shader's `smoothstep` never gets a span of zero width.
        (0.2, 0.0, 0.999),
        // The whole arm, fading from the crossing out.
        (0.2, 0.2, 0.0),
        // A taper wider than the arm it ends is the same picture as one
        // exactly as wide, rather than a negative share or a NaN.
        (0.2, 0.9, 0.0),
        (0.2, -1.0, 0.999),
        // No arm at all: `derive_pluses` draws nothing here, so what matters is
        // that asking costs no division by zero.
        (0.0, 0.5, 0.999),
    ] {
        let view = ViewConfig { plus_arm: size, plus_taper: taper, ..plus_view() };
        let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
        assert!(
            (scene.plus_taper_start - want).abs() < 1e-5,
            "a {taper} taper on a {size} arm starts at {}, wanted {want}",
            scene.plus_taper_start,
        );
    }
}

/// A marker's two PROPORTIONS are view-wide and reach the renderer through a
/// uniform, so what is asked here is that each arrives and that neither is a
/// size bar wearing a proportion's clothes: same positions, same reach, same
/// grey, whichever way they are dialled.
#[test]
fn neither_proportion_moves_a_marker_or_changes_how_far_it_reaches() {
    let plain = plus_view();
    for (label, dialled) in [
        ("a taper", ViewConfig { plus_taper: 0.2, ..plain.clone() }),
        ("a width", ViewConfig { plus_width: 0.5, ..plain.clone() }),
    ] {
        let a = scene_of(&NoteTracker::new(), &Tuning::default(), &plain, &plain_frame(), 0.0);
        let b = scene_of(&NoteTracker::new(), &Tuning::default(), &dialled, &plain_frame(), 0.0);
        assert!(
            a.plus_taper_start != b.plus_taper_start || a.plus_half_width != b.plus_half_width,
            "{label} never reached the scene, so the rest of this proves nothing",
        );
        assert_eq!(a.pluses.len(), b.pluses.len(), "{label} changed WHICH positions are marked");
        for (x, y) in a.pluses.iter().zip(&b.pluses) {
            assert_eq!(x.pos, y.pos, "{label} moved a marker");
            assert_eq!(x.radius, y.radius, "{label} eats INTO an arm, it never resizes one");
            assert_eq!(x.color, y.color, "{label} changed the grey");
            assert_eq!(x.strength, y.strength);
        }
    }
}

/// The handoff between a name and the marker under it is CONTINUOUS: what the
/// name gives up, the marker takes, in the same frame.
///
/// This is the one place the two claims on a position cross, and a predicate
/// gets it wrong in a way no still picture shows. Under `Played` a name is
/// drawn at exactly the node's activation, so the end of a release is a name
/// too faint to see — and a marker that waits for the name to be gone entirely
/// stays away through all of it and then arrives at FULL opacity the frame
/// activation reaches 0. Measured on the fixture below before this was a
/// level: at an activation of 0.0005 the position held a name at 0.05% and no
/// marker at all, and one frame later a marker at 1.0. A hole, then a pop, once
/// per note.
///
/// A 2-second fade sampled along it, rather than at the ends, because the ends
/// are exactly where both spellings agree.
#[test]
fn a_marker_takes_back_what_a_names_fade_gives_up() {
    let view = ViewConfig { note_names: NoteNames::Played, ..plus_view() };
    let frame = FrameParams { fade_time: 2.0, ..FrameParams::default() };
    let mut tracker = NoteTracker::new();
    tracker.handle_event(NoteEvent::on(0.0, 0, 60, 1.0));
    tracker.handle_event(NoteEvent {
        time: 4.0,
        channel: 0,
        note: 60,
        kind: harmonigraph_core::NoteEventKind::Off,
    });

    let mut walk = vec![];
    for now in [4.0f64, 5.0, 5.8, 5.98, 5.999, 6.0, 6.5] {
        let scene = scene_of(&tracker, &Tuning::default(), &view, &frame, now);
        let node = origin_node(&scene);
        let ground = scene.lattice_ground.w;
        let standing = scene
            .pluses
            .iter()
            .find(|p| p.pos == node.world_pos)
            .map_or(0.0, |p| p.strength);
        // The complement, exactly: the name's level under Played IS the
        // activation (`label_strength` in harmonigraph-ui), so the two together
        // are one whole marker's worth of ink at every instant.
        let want = ground * (1.0 - node.name_level(&view));
        assert!(
            (standing - want).abs() < 1e-5,
            "at {now}s the name stands at {} and the marker at {standing}, wanted {want}",
            node.name_level(&view),
        );
        walk.push(standing);
    }

    // No step anywhere along it, and in particular not across the frame the
    // note finally stops sounding — which is the one the predicate jumped on.
    for pair in walk.windows(2) {
        assert!(
            (pair[1] - pair[0]).abs() < 0.55,
            "the marker jumps through the release rather than fading in: {walk:?}",
        );
    }
    assert!(walk[0] < 1e-5, "a sounding note's own name leaves no marker under it");
    assert!(
        (walk[walk.len() - 1] - walk[walk.len() - 2]).abs() < 1e-5,
        "and silence is where the fade ARRIVES, not where it starts: {walk:?}",
    );
}

/// A sounding note claims the position it stands on, names or no names.
///
/// A marker's cross writes a standoff into the light (`plus_standoff` in
/// lattice.wgsl), and the middle of a node is the one place the picture keeps
/// free of one: inside the innermost ring nothing stands the light off, which
/// is what lights the middle of a node whose innermost ring is an annulus
/// ([`ViewConfig::glow_gap`]). A marker standing at the node's own centre cuts
/// a cross-shaped bite out of that node's halo — measured at the fresh glow
/// bars as a dark plus in every lit node, and gone when the standoff is.
///
/// Switching the Note names OFF is the whole of what reaches it, and is why
/// this holds nothing about the fresh picture: under every Show mode a name
/// already covers a sounding note, so there the two claims agree and the
/// handoff is the one [`a_marker_takes_back_what_a_names_fade_gives_up`]
/// measures. It is asked of whichever claims the position harder rather than
/// of the two multiplied, which is what keeps that agreement exact.
#[test]
fn a_sounding_note_takes_the_marker_under_it_with_the_names_off() {
    let view = ViewConfig { show_labels: false, ..plus_view() };
    let frame = FrameParams { fade_time: 2.0, ..FrameParams::default() };
    let mut tracker = NoteTracker::new();
    tracker.handle_event(NoteEvent::on(0.0, 0, 60, 1.0));
    tracker.handle_event(NoteEvent {
        time: 4.0,
        channel: 0,
        note: 60,
        kind: harmonigraph_core::NoteEventKind::Off,
    });

    let mut walk = vec![];
    for now in [4.0f64, 5.0, 5.8, 5.999, 6.0, 6.5] {
        let scene = scene_of(&tracker, &Tuning::default(), &view, &frame, now);
        let node = origin_node(&scene);
        let standing = scene
            .pluses
            .iter()
            .find(|p| p.pos == node.world_pos)
            .map_or(0.0, |p| p.strength);
        assert!(
            (standing - (1.0 - node.activation)).abs() < 1e-5,
            "at {now}s the note stands at {} and the marker at {standing}",
            node.activation,
        );
        walk.push(standing);
    }
    assert!(walk[0] < 1e-5, "a marker stands in the middle of a sounding node's halo");
    for pair in walk.windows(2) {
        assert!(
            (pair[1] - pair[0]).abs() < 0.55,
            "the marker jumps back in rather than fading: {walk:?}",
        );
    }

    // Every home position keeps its marker but the ones the note is sounding
    // at, which under a 12-TET tuning is every spelling of its pitch class
    // rather than the one it was struck as.
    let scene = scene_of(&tracker, &Tuning::default(), &view, &frame, 4.0);
    let home = scene.nodes.iter().filter(|n| n.on_home).count();
    let lit = scene.nodes.iter().filter(|n| n.on_home && n.activation > 0.0).count();
    assert!(lit > 0 && lit < home, "the fixture must light some of the field and not all of it");
    assert_eq!(scene.pluses.len(), home - lit, "the note took a marker it is not sounding at");
}

#[test]
fn an_unlit_node_carries_the_idle_grey_and_draws_nothing() {
    // An idle node has no mark of its own: the marker standing at its position
    // is what says the position is there. `color` is what a node with no
    // voice on it falls back to, and nothing draws while it holds that -- so
    // this pins the neutral rather than a look, and pins that the trail never
    // overwrites it (see the trail tests).
    //
    // The GROUND, which is the marker's own colour: a node arriving or leaving
    // has to cross no seam against the marker under it.
    let view = plus_view();
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &view,
        &plain_frame(),
        0.0,
    );
    let idle = scene
        .nodes
        .iter()
        .find(|n| n.activation == 0.0)
        .expect("nothing is playing");
    assert_eq!(idle.color, scene.lattice_ground);
    assert_eq!(
        idle.color,
        crate::grey_of_lightness(view.lattice_ground_lightness()),
        "the fallback is not the grey the Ground bar names",
    );
    assert!(
        scene.nodes.iter().all(|n| n.activation == 0.0),
        "nothing sounds, so every node is idle",
    );
}

#[test]
fn a_resting_marker_is_the_grey_its_own_bar_names() {
    // A marker at rest IS the grey the Marker ink bar names — not a grey near
    // it, and not a brightness of one. Held at three settings, because one
    // would pass against a marker that had simply been re-pinned to some fixed
    // grey.
    //
    // The OPACITY is half the claim and the easier half to lose: `strength`
    // premultiplies the colour, so a marker carrying an alpha of its own lands
    // on a blend of that grey and whatever is behind it — a different colour
    // per background, and none of them the one asked for. A mark drawn at a
    // chrome opacity is that alternative, and it is why such a mark and a bar's
    // number can only ever nearly agree.
    for ink in [0.0f32, 20.0, 64.0] {
        let view = ViewConfig { marker_ink: ink, ..plus_view() };
        let pluses = pluses_of(&view);
        let resting = pluses.first().expect("the home sheet draws a resting marker field");
        assert_eq!(
            resting.color,
            crate::grey_of_lightness(ink),
            "at Marker ink {ink} a resting marker is not the grey the bar names",
        );
        assert_eq!(
            resting.strength, 1.0,
            "at Marker ink {ink} a resting marker carries an alpha, so what it \
             draws is a blend rather than the grey",
        );
    }
}

/// The node's two at-rest surfaces measured against EACH OTHER, through one
/// derive: the audio ring's silent end and what an unplayed node falls back to
/// are one colour.
///
/// The one test that fails if either is re-pinned to a grey of its own — which
/// is the shape the bug takes, each surface aimed at the other by hand and
/// landing a hair off. The markers are deliberately NOT in it; see
/// [`the_two_at_rest_bars_move_nothing_of_each_others`] for the claim that
/// replaces their membership.
#[test]
fn the_ring_and_an_idle_node_are_one_grey() {
    for ground in [8.0f32, 20.0, 45.0] {
        let view = ViewConfig { lattice_ground: ground, ..plus_view() };
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &view,
            &plain_frame(),
            0.0,
        );
        let idle = scene
            .nodes
            .iter()
            .find(|n| n.activation == 0.0)
            .expect("nothing is playing");
        let ring = crate::SpectralPaint::new(&view, crate::Gradient::default()).lut[0];
        for (what, got) in [("an idle node", idle.color), ("the audio ring", ring)] {
            let step = (got.truncate() - scene.lattice_ground.truncate()).abs().max_element();
            assert!(
                step * 255.0 < 0.5,
                "at Ground {ground} {what} draws {got:?} against the ground's {:?}",
                scene.lattice_ground,
            );
        }
    }
}

/// A FRESH lattice draws its whole resting picture in one grey: the two at-rest
/// bars open on the same `L*`, so nothing about a fresh install says the
/// markers and the rings are dialled separately.
///
/// The pair is what a person is meant to compare, and a fresh view that opened
/// them apart would be the panel arguing for a split before anyone asked for
/// one. It is also the difference between a second bar and a changed picture —
/// retuning one fresh value and not the other is exactly the drift this
/// catches, and it is invisible in every other test here, all of which set both
/// bars.
#[test]
fn a_fresh_lattice_rests_in_one_grey() {
    let fresh = ViewConfig::default();
    assert_eq!(
        fresh.marker_ink, fresh.lattice_ground,
        "a fresh lattice opens with its markers and its unlit rings on two greys",
    );
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &ViewConfig { extent_threes: 2, extent_fives: 2, ..fresh },
        &plain_frame(),
        0.0,
    );
    let marker = scene.pluses.first().expect("the home sheet draws a resting marker field");
    assert_eq!(
        marker.color, scene.lattice_ground,
        "the fresh bars agree and the picture still draws two greys",
    );
}

/// The markers and the node's unlit rings are dialled by two bars, and neither
/// bar reaches the other's picture.
///
/// Both directions, because the coupling can survive in either one and each is
/// invisible from the other side: markers still riding the Ground is the state
/// the second bar was added to leave, and a node's rings drifting with the
/// Marker ink is what a single shared resolve handed to the wrong consumer
/// looks like.
///
/// Every pair of two settings, so a picture that simply averaged the two would
/// fail: at (0, 64) the markers must be black against a light ring and at
/// (64, 0) the reverse, which no coupled value can do.
#[test]
fn the_two_at_rest_bars_move_nothing_of_each_others() {
    for ground in [0.0f32, 64.0] {
        for ink in [0.0f32, 64.0] {
            let view = ViewConfig { lattice_ground: ground, marker_ink: ink, ..plus_view() };
            let scene = scene_of(
                &NoteTracker::new(),
                &Tuning::default(),
                &view,
                &plain_frame(),
                0.0,
            );
            let marker =
                scene.pluses.first().expect("the home sheet draws a resting marker field");
            assert_eq!(
                marker.color,
                crate::grey_of_lightness(ink),
                "at Ground {ground} / Marker ink {ink} a marker draws {:?}",
                marker.color,
            );
            assert_eq!(
                scene.lattice_ground,
                crate::grey_of_lightness(ground),
                "at Ground {ground} / Marker ink {ink} the rings stand on {:?}",
                scene.lattice_ground,
            );
            // The audio ring's table, the ground's reader one crate away and
            // the one a marker's bar could reach by mistake: it is baked from
            // the Ground alone, so an ink bar wired into `SpectralPaint` would
            // show up here and nowhere else.
            let silent = crate::SpectralPaint::new(&view, crate::Gradient::default()).lut[0];
            let step = (silent.truncate() - scene.lattice_ground.truncate()).abs().max_element();
            assert!(
                step * 255.0 < 0.5,
                "at Ground {ground} / Marker ink {ink} the audio ring's silence is \
                 {silent:?} against the ground's {:?}",
                scene.lattice_ground,
            );
        }
    }
}

#[test]
fn a_marker_that_survives_a_chord_is_painted_exactly_as_it_was() {
    // A marker's own PAINT is purely the resting picture: no note tints one,
    // lights one, moves one or resizes one. Which positions are marked is a
    // separate question and a note does move that — a sounding node is named,
    // and a named position draws no marker — so this walks the survivors by
    // POSITION rather than by index and says nothing about how many there are.
    //
    // Just intonation and a small window so pitch classes stay unique.
    let tuning = Tuning { tolerance: harmonigraph_core::tuning::microcents(5.0), ..Tuning::just() };
    let mut tracker = NoteTracker::new();
    for note in [60u8, 67] {
        tracker.handle_event(NoteEvent::on(0.0, 0, note, 1.0));
    }
    let view = ViewConfig { extent_threes: 3, extent_fives: 3, ..plus_view() };
    let silent = pluses_of(&view);
    let played = pluses_of_with(&view, &tracker);
    assert!(
        played.len() < silent.len(),
        "the chord has to take some pluses for this to mean anything",
    );
    for b in &played {
        let a = silent
            .iter()
            .find(|a| a.pos == b.pos)
            .expect("a marker appeared at a position that had none while silent");
        assert_eq!(a.color, b.color, "a chord tinted a marker: {b:?}");
        assert_eq!(a.strength, b.strength, "a chord lit a marker: {b:?}");
        assert_eq!(a.radius, b.radius, "a chord resized a marker: {b:?}");
    }
    // And every survivor stands on the plain ground.
    let scene = scene_of(&tracker, &tuning, &view, &plain_frame(), 0.0);
    for marker in &scene.pluses {
        assert_eq!(marker.color, scene.lattice_ground, "{marker:?}");
        assert_eq!(marker.strength, 1.0, "{marker:?}");
    }
    assert!(
        scene.nodes.iter().any(|n| n.activation > 0.0),
        "the chord has to reach the lattice for this to mean anything",
    );
}

#[test]
fn a_named_position_draws_no_marker() {
    // Two markers claiming one position is the thing this rule is for, and the
    // name is the better of the two: it says WHICH position, where the marker only
    // says that there is one. Held across all three Show modes at once, because
    // what counts as "named at rest" is the whole of what separates them.
    let view = ViewConfig { note_names: NoteNames::Past, ..plus_view() };
    let tracker = played_and_forgotten();
    let scene = scene_of(&tracker, &Tuning::default(), &view, &plain_frame(), 4.0);
    let remembered: Vec<&NodeInstance> =
        scene.nodes.iter().filter(|n| n.trail > 0.0).collect();
    assert!(!remembered.is_empty(), "nothing was remembered, so nothing is named at rest");
    for node in &remembered {
        assert!(
            !scene.pluses.iter().any(|d| d.pos == node.world_pos),
            "the remembered position {:?} wears both a name and a marker",
            node.lattice_pos,
        );
    }
    // And the positions with no memory on them keep theirs — the rule is about
    // the name, not about the mode being on.
    let unvisited = scene
        .nodes
        .iter()
        .filter(|n| n.on_home && n.trail == 0.0 && n.activation == 0.0)
        .count();
    assert_eq!(scene.pluses.len(), unvisited, "an unnamed home position lost its marker");

    // All names every node on screen, so the field goes with it — the mode
    // working as it reads rather than a case to special-case.
    let all = pluses_of(&ViewConfig { note_names: NoteNames::All, ..view.clone() });
    assert!(all.is_empty(), "All names every node, so no marker should survive: {all:?}");

    // Played names nothing at rest, so every resting position keeps its marker.
    let played = scene_of(
        &tracker,
        &Tuning::default(),
        &ViewConfig { note_names: NoteNames::Played, ..view.clone() },
        &plain_frame(),
        4.0,
    );
    let home = played.nodes.iter().filter(|n| n.on_home).count();
    let why = "Played names nothing at rest, so nothing loses a marker";
    assert_eq!(played.pluses.len(), home, "{why}");
}

#[test]
fn a_hovered_position_draws_no_marker() {
    // Hovering is the one way a name arrives under every mode, Played
    // included, so it is the case that proves the marker answers to the NAME
    // rather than to the Show row.
    let view = ViewConfig { note_names: NoteNames::Played, ..plus_view() };
    let at = LatticePos::ORIGIN;
    let scene = crate::derive_scene(
        &NoteTracker::new(),
        &Tuning::default(),
        &view,
        &view.reach(),
        &plain_frame(),
        crate::Camera::default(),
        Some(at),
        0.0,
    );
    let hovered = scene
        .nodes
        .iter()
        .find(|n| n.hovered)
        .expect("the pointer is on the origin");
    assert!(
        !scene.pluses.iter().any(|d| d.pos == hovered.world_pos),
        "the hovered position wears both a name and a marker",
    );
    assert!(!scene.pluses.is_empty(), "and only that one lost it");
}

#[test]
fn names_switched_off_leave_every_marker_standing() {
    // The rule is "a name is over it", not "a name would be over it if names
    // were on". With the Note names switch off there is no name anywhere, so
    // the field is whole under every Show mode — including All, which is the
    // one that would otherwise erase it.
    for names in [NoteNames::All, NoteNames::Past, NoteNames::Played] {
        let view = ViewConfig { show_labels: false, note_names: names, ..plus_view() };
        let tracker = played_and_forgotten();
        let scene = scene_of(&tracker, &Tuning::default(), &view, &plain_frame(), 4.0);
        let home = scene.nodes.iter().filter(|n| n.on_home).count();
        assert_eq!(
            scene.pluses.len(),
            home,
            "{names:?} with the names switched off still took a marker away",
        );
    }
}

#[test]
fn an_off_sheet_note_leaves_the_marker_field_alone() {
    // A 7-limit note sounding off the home sheet hangs from nothing: the
    // sheet it left is marked and the one it is on is not, and the SIZE it
    // draws at is what says how far off it has gone. A chain drawn down to
    // home would be the other answer, and the field is what it costs — so the
    // field is the same field whether that note sounds or not.
    let view = ViewConfig {
        extent_threes: 0,
        extent_fives: 0,
        extent_sevens: 2,
        ..plain_view()
    };
    // 12-TET default: a sevens step is 1000¢, so (0,0,2) is MIDI 68's pitch
    // class. The home node is C.
    let mut tracker = NoteTracker::new();
    tracker.handle_event(NoteEvent::on(0.0, 0, 68, 1.0));
    let scene = scene_of(&tracker, &Tuning::default(), &view, &plain_frame(), 0.0);
    assert!(
        scene.nodes.iter().any(|n| !n.on_home && n.activation > 0.0),
        "the off-sheet note has to be lit for this to mean anything",
    );
    assert_eq!(
        scene.pluses.len(),
        1,
        "a column one node wide has one home position, so one marker: {:?}",
        scene.pluses,
    );
    assert!(
        scene.pluses[0].pos.z.abs() < 1e-5,
        "and it is the home sheet's: {:?}",
        scene.pluses[0],
    );
}
