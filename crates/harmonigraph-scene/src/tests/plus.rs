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
fn a_resting_marker_is_the_lattices_own_ground() {
    // The markers and the rings are one resting picture, so a marker at rest IS
    // the ground the rings stand on — not a grey near it. Held at three
    // settings of the bar, because one would pass against a marker that had
    // simply been re-pinned to some fixed grey.
    //
    // The OPACITY is half the claim and the easier half to lose: `strength`
    // premultiplies the colour, so a marker carrying an alpha of its own lands
    // on a blend of the ground and whatever is behind it — a different grey
    // per background, and none of them this one. A mark drawn at a chrome
    // opacity is that alternative, and it is why such a mark and the rings
    // can only ever nearly agree.
    for ground in [0.0f32, 20.0, 64.0] {
        let view = ViewConfig { lattice_ground: ground, ..plus_view() };
        let pluses = pluses_of(&view);
        let resting = pluses.first().expect("the home sheet draws a resting marker field");
        assert_eq!(
            resting.color,
            crate::grey_of_lightness(ground),
            "at Ground {ground} a resting marker is not the grey the bar names",
        );
        assert_eq!(
            resting.strength, 1.0,
            "at Ground {ground} a resting marker carries an alpha, so what it \
             draws is a blend rather than the ground",
        );
    }
}

/// The three at-rest surfaces measured against EACH OTHER, through one derive:
/// a marker, the audio ring's silent end, and what an unplayed node falls back to
/// are one colour.
///
/// The whole ask, and the one test that fails if any of the three is re-pinned
/// to a grey of its own — which is the shape the bug takes, each surface aimed
/// at the others by hand and landing a hair off.
#[test]
fn the_markers_the_ring_and_an_idle_node_are_one_grey() {
    for ground in [8.0f32, 20.0, 45.0] {
        let view = ViewConfig { lattice_ground: ground, ..plus_view() };
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &view,
            &plain_frame(),
            0.0,
        );
        let marker = scene.pluses.first().expect("the home sheet draws a resting marker field");
        let idle = scene
            .nodes
            .iter()
            .find(|n| n.activation == 0.0)
            .expect("nothing is playing");
        let ring = crate::SpectralPaint::new(&view, crate::Gradient::default()).lut[0];
        for (what, got) in
            [("a marker", marker.color), ("an idle node", idle.color), ("the audio ring", ring)]
        {
            let step = (got.truncate() - scene.lattice_ground.truncate()).abs().max_element();
            assert!(
                step * 255.0 < 0.5,
                "at Ground {ground} {what} draws {got:?} against the ground's {:?}",
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
    // A 7-limit note sounding off the home sheet used to hang from a dashed
    // chain drawn down to it. It does not any more: the sheet it left is
    // marked and the one it is on is not, and the SIZE it draws at is what
    // says how far off it has gone. So the field is the same field whether
    // that note sounds or not.
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
