//! The lattice structure standing AT the nodes: which positions carry a marker,
//! how big it is, and the independent grey it uses at rest.
//!
//! Nothing is drawn BETWEEN the positions, and several tests here exist to
//! hold that: the field's regularity is what the rows and columns are read
//! off, so the resting picture is a set of marks and not a mesh.

use super::harness::*;
use crate::*;
use harmonigraph_core::{LatticePos, NoteEvent, NoteTracker, SourceId, Tuning};

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
        min_sevens: -1,
        max_sevens: 1,
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
        tracker.handle_event(NoteEvent {
            source: SourceId::DIRECT,
            time,
            channel: 0,
            note: 60,
            kind,
        });
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

fn unit_of(view: &ViewConfig) -> f32 {
    scene_of(&NoteTracker::new(), &Tuning::default(), view, &plain_frame(), 0.0).marker_unit
}

/// `marker_unit` converts the marker field between the world its draws are in
/// and the quad uv its bars are dialled in — an arm reads back as the number
/// the bar behind it holds.
///
/// The shader's own reason for wanting it: a marker's draw is handed world
/// lengths (a billboard, and an arm per instance) and has to grow its quad by
/// the Shadow's own reach, which is a node's bar and so in uv. One of the two
/// has to be converted, and the conversion is the SCENE's rather than any
/// marker's — carrying it over is what keeps `lattice.wgsl` from spelling the
/// uv rule a second time for one more layer.
///
/// Read against the bars rather than against a formula, which is the whole
/// point: a unit that disagreed with them would leave the marker's quad grown
/// by a different Shadow than every node's, on one bar.
#[test]
fn the_marker_unit_is_what_reads_a_markers_world_back_as_its_bars() {
    let view = ViewConfig { plus_arm: 0.2, ..plus_view() };
    let unit = unit_of(&view);
    assert!(unit > 0.0, "the field must have a unit to be measured in");
    let arm = pluses_of(&view)[0].radius;
    assert!((arm / unit - 0.2).abs() < 1e-4, "an arm of 0.2 read back as {}", arm / unit,);
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
    let expected: Vec<_> =
        scene.nodes.iter().enumerate().filter(|(_, n)| n.on_home).map(|(i, _)| i).collect();
    assert_eq!(scene.pluses.iter().map(|p| p.node).collect::<Vec<_>>(), expected);
    for marker in &scene.pluses {
        let node = &scene.nodes[marker.node];
        assert!(node.on_home);
        assert_eq!(marker.pos, node.world_pos);
    }
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
        let node = &scene.nodes[marker.node];
        assert!(node.on_home);
        assert_eq!(marker.pos, node.world_pos);
        assert!(
            scene.nodes.iter().any(|n| n.world_pos == marker.pos),
            "a panned marker at {:?} stands at no node",
            marker.pos,
        );
    }
}

#[test]
fn nothing_is_drawn_between_two_positions() {
    // A marker stands ON a node, so every one of them is at a position the
    // window holds — and a mark at the midpoint of two of them would be an
    // interval drawn, which is exactly what a mesh says and a field does not.
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

/// An arm that is not a real number takes the field away, the way 0 does —
/// and takes it away through the SAME `radius <= 0.0` test, because `size`
/// answers every non-finite arm with 0 before a radius is built from it.
///
/// Which is why this pins the door and not only the picture. `derive_pluses`
/// carries no NaN branch of its own and cannot: with `size` the only way in,
/// there is nothing left to reach one. A radius assembled from some later
/// factor that `size` has NOT been over would put a NaN past the off test —
/// NaN answers no to `<= 0.0` the way it answers no to every comparison — and
/// ship the field whole at a size the shader cannot draw, the resting
/// structure gone with nothing on screen saying why. The lower assertion is
/// what goes red when that door is the one that moved.
///
/// The arm has a repair at the blob's door; this is the picture's own,
/// for shells that never cross it.
#[test]
fn a_marker_radius_that_is_not_a_number_takes_the_field_away() {
    for arm in [f32::NAN, f32::NEG_INFINITY] {
        let view = ViewConfig { plus_arm: arm, ..plus_view() };
        let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
        // The fixture must reach a position for a marker to stand at.
        assert!(
            scene.nodes.iter().any(|n| n.on_home),
            "an arm of {arm} left no home position to mark, so the field below proves nothing",
        );
        assert!(scene.pluses.is_empty(), "an arm of {arm} shipped a marker field");
        // The door itself, so that the empty field above stays evidence of a
        // repair rather than of a coincidence downstream.
        assert_eq!(
            crate::view::size(arm, PLUS_SIZE_MAX),
            0.0,
            "`size` passed an arm of {arm} through, so the off test alone is no longer the guard",
        );
    }
}

/// A cross's thickness follows the label scale, as a letter's stroke does: the
/// half-width the shader reads is proportional to it, and at the default scale
/// it is exactly what the retired width bar's captured default drew.
#[test]
fn the_label_scale_sets_the_cross_width() {
    let arm = ViewConfig::default().plus_arm;
    let half_at = |label_scale: f32| {
        let view = ViewConfig { plus_arm: arm, label_scale, ..plus_view() };
        scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0)
            .plus_half_width
    };
    // At label scale 1 the whole width is the constant, halved and taken as a
    // share of the arm.
    assert!((half_at(1.0) - PLUS_WIDTH_PER_LABEL_SCALE * 0.5 / arm).abs() < 1e-6);
    // Proportional, so twice the label scale is twice the thickness.
    let (one, two) = (half_at(1.0), half_at(2.0));
    assert!((two - 2.0 * one).abs() < 1e-6, "scale 1 drew {one}, scale 2 drew {two}");
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
        ("a label scale", ViewConfig { label_scale: 2.0, ..plain.clone() }),
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
    tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
    tracker.handle_event(NoteEvent {
        source: SourceId::DIRECT,
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
        let standing =
            scene.pluses.iter().find(|p| p.pos == node.world_pos).map_or(0.0, |p| p.strength);
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

/// The audio ring's silent end agrees with the lattice ground at every setting.
#[test]
fn the_silent_ring_and_lattice_ground_are_one_grey() {
    for ground in [8.0f32, 20.0, 45.0] {
        let view = ViewConfig { lattice_ground: ground, ..plus_view() };
        let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
        let ring = crate::SpectralPaint::new(&view, crate::Gradient::default()).lut[0];
        let step = (ring.truncate() - scene.lattice_ground.truncate()).abs().max_element();
        assert!(
            step * 255.0 < 0.5,
            "at Ground {ground} the audio ring draws {ring:?} against the ground's {:?}",
            scene.lattice_ground,
        );
    }
}

/// A fresh lattice holds its marker field above the ring ground, and the scene
/// spends each bar on its own surface.
///
/// The split keeps the resting positions legible through the broad node glow.
/// Both the values and their consumers are part of the look: equal defaults or
/// a crossed wire would flatten the pair back to one grey.
#[test]
fn a_fresh_lattice_holds_its_markers_above_the_ring_ground() {
    let fresh = ViewConfig::default();
    assert!(
        fresh.marker_ink > fresh.lattice_ground,
        "the fresh marker ink {} does not stand above the ring ground {}",
        fresh.marker_ink,
        fresh.lattice_ground,
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
        marker.color,
        crate::grey_of_lightness(fresh.marker_ink),
        "the fresh marker does not draw at its own ink",
    );
    assert_eq!(scene.lattice_ground, crate::grey_of_lightness(fresh.lattice_ground));
    assert_ne!(marker.color, scene.lattice_ground, "the fresh pair collapsed to one grey");
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
            let scene =
                scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
            let marker = scene.pluses.first().expect("the home sheet draws a resting marker field");
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
        tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, note, 1.0));
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
    // And every survivor keeps the marker field's own resting ink.
    let scene = scene_of(&tracker, &tuning, &view, &plain_frame(), 0.0);
    for marker in &scene.pluses {
        assert_eq!(marker.color, crate::grey_of_lightness(view.marker_ink), "{marker:?}");
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
    let remembered: Vec<&NodeInstance> = scene.nodes.iter().filter(|n| n.trail > 0.0).collect();
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
    let unvisited =
        scene.nodes.iter().filter(|n| n.on_home && n.trail == 0.0 && n.activation == 0.0).count();
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
    let mut scene = crate::derive_scene(
        &NoteTracker::new(),
        &Tuning::default(),
        &view,
        &view.reach(),
        &plain_frame(),
        crate::Camera::default(),
        Some(at),
    );
    NodeMotion::default().step(
        &mut scene,
        &NoteTracker::new(),
        &Tuning::default(),
        &view,
        &view.envelope(&plain_frame()),
        &RingFade::default(),
        0.0,
    );
    let hovered = scene.nodes.iter().find(|n| n.hovered).expect("the pointer is on the origin");
    assert!(
        !scene.pluses.iter().any(|d| d.pos == hovered.world_pos),
        "the hovered position wears both a name and a marker",
    );
    assert!(!scene.pluses.is_empty(), "and only that one lost it");
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
        min_sevens: -2,
        max_sevens: 2,
        ..plain_view()
    };
    // 12-TET default: a sevens step is 1000¢, so (0,0,2) is MIDI 68's pitch
    // class. The home node is C.
    let mut tracker = NoteTracker::new();
    tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 68, 1.0));
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

/// A node lit by something that is not a key keeps its cross whole, and the
/// light standing over it moves the cross's SHADOW by nothing either.
///
/// A marker's ink answers to what is DRAWN at the position, and an analyzer
/// ring — or a halo left over from a note the position has already handed
/// back — is light a node WEARS rather than a claim on the position under it.
/// So the cross stands, and stands whole: measured against a resting neighbour
/// rather than against a number, so what is claimed is about the light and not
/// about whatever `marker_ink` happens to be dialled to.
///
/// The shadow is the same claim, and it is structural rather than arithmetic: a
/// marker hands the picture ONE number ([`PlusInstance::strength`]), spent on
/// its ink and on the share of the shadow its cross casts alike. A light term
/// anywhere in here is what this catches, by handing the derivation a fully lit
/// node and a dark one and requiring the same field back.
#[test]
fn a_node_lit_by_no_key_keeps_its_cross_whole() {
    let view = plus_view();
    let mut scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
    let lit = origin_node(&scene).lattice_pos;
    let dark = scene
        .nodes
        .iter()
        .find(|n| n.on_home && n.lattice_pos != lit)
        .expect("the window must hold a second home position")
        .lattice_pos;
    for plus in &scene.pluses {
        assert!(plus.strength > 0.0, "an unplayed field draws its markers: {plus:?}");
    }

    // What a ring's carried light looks like by the time the marker field is
    // asked: a node fully lit with an activation of 0 under it. The ink the
    // field is derived at is arbitrary — the claim is a difference, not a grey.
    let ink = glam::Vec4::new(0.5, 0.5, 0.5, 0.75);
    let unlit = crate::derive::derive_pluses(&view, &scene.nodes, ink);
    for node in &mut scene.nodes {
        if node.lattice_pos == lit {
            node.glow.level = 1.0;
        }
    }
    assert!(
        origin_node(&scene).activation < 1e-5,
        "the fixture must light the node without a key, or it proves nothing",
    );
    let shining = crate::derive::derive_pluses(&view, &scene.nodes, ink);

    let at = |field: &[PlusInstance], pos: LatticePos| {
        *field
            .iter()
            .find(|p| scene.nodes[p.node].lattice_pos == pos)
            .expect("every home position keeps one")
    };
    assert_eq!(
        unlit.len(),
        shining.len(),
        "lighting a node took a marker out of the field: {} against {}",
        unlit.len(),
        shining.len(),
    );
    assert!(
        (at(&shining, lit).strength - at(&unlit, lit).strength).abs() < 1e-5,
        "a light over a position moved the marker standing at it: {:?} against {:?}",
        at(&shining, lit),
        at(&unlit, lit),
    );
    assert!(
        (at(&shining, lit).strength - at(&shining, dark).strength).abs() < 1e-5,
        "a lit node's cross is not the one every resting position wears: {:?} against {:?}",
        at(&shining, lit),
        at(&shining, dark),
    );
    assert!(at(&shining, lit).strength > 0.0, "and the field has to be drawing markers at all");
}

/// A cross and its shadow come back together: through a release the marker's
/// one number rides the NAME's clock and nothing else's.
///
/// The two clocks the picture has here are different lengths on purpose —
/// `Voice::activation` reaches exactly 0 at the Fade, and the light is a
/// first-order lag on the Glow release running well past it. A shadow closed
/// against the light rather than against the ink is a cross that fades back in
/// over the Fade with nothing under it, and a shadow that then eases in over
/// the seconds after, on a clock nothing on screen explains. What is measured
/// here is the ramp itself: at every point of the release the marker is worth
/// exactly the share of the position its name has handed back.
#[test]
fn a_markers_shadow_fades_in_with_its_cross() {
    let view = plus_view();
    let frame = FrameParams { fade_time: 2.0, ..FrameParams::default() };
    let mut tracker = NoteTracker::new();
    tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
    tracker.handle_event(NoteEvent {
        source: SourceId::DIRECT,
        time: 4.0,
        channel: 0,
        note: 60,
        kind: harmonigraph_core::NoteEventKind::Off,
    });

    // The resting grey, off a position no note has been near: what a marker is
    // worth once the name over it is gone.
    let whole = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &frame, 0.0)
        .pluses
        .first()
        .expect("an unplayed field draws its markers")
        .strength;
    assert!(whole > 0.0, "the fixture must draw markers at all");

    let mut last = 0.0f32;
    for now in [4.5f64, 5.0, 5.5, 6.0, 7.0] {
        let scene = scene_of(&tracker, &Tuning::default(), &view, &frame, now);
        let node = *origin_node(&scene);
        let want = whole * (1.0 - node.name_level(&view));
        let held = scene
            .pluses
            .iter()
            .find(|p| scene.nodes[p.node].lattice_pos == LatticePos::ORIGIN)
            .map_or(0.0, |p| p.strength);
        assert!(
            (held - want).abs() < 1e-5,
            "at {now}s a name of {} left {held} of a {whole} marker, not {want}",
            node.name_level(&view),
        );
        assert!(held >= last, "the cross went backwards at {now}s: {held} after {last}");
        last = held;
    }
    assert!((last - whole).abs() < 1e-5, "the cross never came all the way back: {last}");
}
