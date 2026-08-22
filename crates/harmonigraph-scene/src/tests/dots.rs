//! The lattice structure standing AT the nodes: which positions carry a dot,
//! how big it is, and the one grey it shares with everything else at rest.
//!
//! Nothing is drawn BETWEEN the positions, and several tests here exist to
//! hold that: the field's regularity is what the rows and columns are read
//! off, so the resting picture is a set of marks and not a mesh.

use crate::*;
use harmonigraph_core::{NoteEvent, NoteTracker, Tuning};
use super::harness::*;

/// A 7x7 window one sevens step deep, so off-sheet positions are in the mix
/// and can be shown to carry no dot.
fn dot_view() -> ViewConfig {
    ViewConfig {
        extent_threes: 3,
        extent_fives: 3,
        extent_sevens: 1,
        ..plain_view()
    }
}

fn dots_of(view: &ViewConfig) -> Vec<DotInstance> {
    dots_of_with(view, &NoteTracker::new())
}

fn dots_of_with(view: &ViewConfig, tracker: &NoteTracker) -> Vec<DotInstance> {
    scene_of(tracker, &Tuning::default(), view, &plain_frame(), 0.0).dots
}

#[test]
fn a_dot_stands_at_every_home_position_and_nowhere_else() {
    // The dot field IS the lattice's resting picture, so "one per home
    // position" is the whole shape of it: a position with no dot is a
    // position nobody can see, and a dot with no position is a mark from
    // nowhere. Both halves, because a count alone passes on a field that has
    // drifted off the nodes entirely.
    let view = dot_view();
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
    let home: Vec<&NodeInstance> = scene.nodes.iter().filter(|n| n.on_home).collect();
    assert!(!home.is_empty() && home.len() < scene.nodes.len(), "want both kinds in the window");
    assert_eq!(scene.dots.len(), home.len(), "one dot per home position, and no others");
    for node in &home {
        assert!(
            scene.dots.iter().any(|d| d.pos == node.world_pos),
            "no dot standing at the home position {:?}",
            node.lattice_pos,
        );
    }
    for node in scene.nodes.iter().filter(|n| !n.on_home) {
        assert!(
            !scene.dots.iter().any(|d| d.pos == node.world_pos),
            "an off-sheet position at {:?} was marked; the home sheet is the ground \
             because it is the only one that draws at rest",
            node.lattice_pos,
        );
    }

    // Panning the window keeps the dots attached to the visible nodes (both
    // are derived in centered world space).
    let panned = ViewConfig { center_threes: 3, ..view };
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &panned, &plain_frame(), 0.0);
    for dot in &scene.dots {
        assert!(
            scene.nodes.iter().any(|n| n.world_pos == dot.pos),
            "a panned dot at {:?} stands at no node",
            dot.pos,
        );
    }
}

#[test]
fn nothing_is_drawn_between_two_positions() {
    // The claim the lines used to make and no longer do. A dot stands ON a
    // node, so every one of them is at a position the window holds — and a
    // mark at the midpoint of two of them would be an interval drawn, which
    // is exactly what a mesh says and a field does not.
    let scene =
        scene_of(&NoteTracker::new(), &Tuning::default(), &dot_view(), &plain_frame(), 0.0);
    for dot in &scene.dots {
        assert!(
            scene.nodes.iter().any(|n| n.world_pos == dot.pos),
            "a dot at {:?} stands between positions rather than at one",
            dot.pos,
        );
    }
}

#[test]
fn dot_size_sets_how_big_a_resting_dot_is_and_0_takes_it_away() {
    // The size bar is the dots' own off switch, and skipping the instances is
    // the same picture the shader would discard to — so 0 has to ship an
    // empty field rather than a field of nothing.
    assert!(
        dots_of(&ViewConfig { dot_size: 0.0, ..dot_view() }).is_empty(),
        "at 0 the dots are off, so no instance should be shipped",
    );

    let mut radii = vec![];
    for size in [0.05f32, 0.2, 0.5, DOT_SIZE_MAX] {
        let dots = dots_of(&ViewConfig { dot_size: size, ..dot_view() });
        let radius = dots[0].radius;
        assert!(
            dots.iter().all(|d| d.radius == radius),
            "every dot in one field is the same size",
        );
        radii.push(radius);
    }
    for pair in radii.windows(2) {
        assert!(pair[1] > pair[0], "more size must mean a bigger dot: {radii:?}");
    }

    // The bar reads in the quad UV a node's ring radii are dialled in, and
    // what reaches the scene is a WORLD length — the conversion is spent once,
    // here, so nothing downstream carries a second copy of the convention.
    let view = ViewConfig { dot_size: 0.5, ..dot_view() };
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
    let want = scene.node_radius * 1.8 * 0.5;
    assert!(
        (scene.dots[0].radius - want).abs() < 1e-5,
        "a uv of 0.5 is {want} in world units, got {}",
        scene.dots[0].radius,
    );
}

#[test]
fn the_feather_reaches_the_scene_clamped() {
    // View-wide rather than per instance, because it is a SHAPE the whole
    // field shares; clamped here rather than in the shader so a hand-edited
    // blob draws a dot somebody can see.
    for (set, want) in [(0.0f32, 0.0f32), (0.4, 0.4), (1.0, 1.0), (5.0, 1.0), (-2.0, 0.0)] {
        let view = ViewConfig { dot_feather: set, ..dot_view() };
        let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
        assert_eq!(
            scene.dot_feather, want,
            "a feather of {set} reached the scene as {}",
            scene.dot_feather,
        );
    }
}

#[test]
fn an_unlit_node_carries_the_idle_grey_and_draws_nothing() {
    // An idle node has no mark of its own: the dot standing at its position
    // is what says the position is there. `color` is what a node with no
    // voice on it falls back to, and nothing draws while it holds that -- so
    // this pins the neutral rather than a look, and pins that the trail never
    // overwrites it (see the trail tests).
    //
    // The GROUND, which is the dot's own colour: a node arriving or leaving
    // has to cross no seam against the dot under it.
    let view = dot_view();
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
fn a_resting_dot_is_the_lattices_own_ground() {
    // The dots and the rings are one resting picture, so a dot at rest IS
    // the ground the rings stand on — not a grey near it. Held at three
    // settings of the bar, because one would pass against a dot that had
    // simply been re-pinned to some fixed grey.
    //
    // The OPACITY is half the claim and the easier half to lose: `strength`
    // premultiplies the colour, so a dot carrying an alpha of its own lands
    // on a blend of the ground and whatever is behind it — a different grey
    // per background, and none of them this one. A mark drawn at a chrome
    // opacity is that alternative, and it is why such a mark and the rings
    // can only ever nearly agree.
    for ground in [0.0f32, 20.0, 64.0] {
        let view = ViewConfig { lattice_ground: ground, ..dot_view() };
        let dots = dots_of(&view);
        let resting = dots.first().expect("the home sheet draws a resting dot field");
        assert_eq!(
            resting.color,
            crate::grey_of_lightness(ground),
            "at Ground {ground} a resting dot is not the grey the bar names",
        );
        assert_eq!(
            resting.strength, 1.0,
            "at Ground {ground} a resting dot carries an alpha, so what it \
             draws is a blend rather than the ground",
        );
    }
}

/// The three at-rest surfaces measured against EACH OTHER, through one derive:
/// a dot, the audio ring's silent end, and what an unplayed node falls back to
/// are one colour.
///
/// The whole ask, and the one test that fails if any of the three is re-pinned
/// to a grey of its own — which is the shape the bug takes, each surface aimed
/// at the others by hand and landing a hair off.
#[test]
fn the_dots_the_ring_and_an_idle_node_are_one_grey() {
    for ground in [8.0f32, 20.0, 45.0] {
        let view = ViewConfig { lattice_ground: ground, ..dot_view() };
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &view,
            &plain_frame(),
            0.0,
        );
        let dot = scene.dots.first().expect("the home sheet draws a resting dot field");
        let idle = scene
            .nodes
            .iter()
            .find(|n| n.activation == 0.0)
            .expect("nothing is playing");
        let ring = crate::SpectralPaint::new(&view, crate::Gradient::default()).lut[0];
        for (what, got) in
            [("a dot", dot.color), ("an idle node", idle.color), ("the audio ring", ring)]
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
fn a_dot_never_moves_with_the_music() {
    // The dots are purely the resting picture. Nothing about them answers to
    // a note — not a dot under a sounding node, not one between two of them,
    // not one on the sheet a played note hangs over. What a note does to the
    // field is COVER its own dot, which is the shader's business and the
    // node's clearing, and nothing derived here.
    //
    // Just intonation and a small window so pitch classes stay unique.
    let tuning = Tuning { tolerance: harmonigraph_core::tuning::microcents(5.0), ..Tuning::just() };
    let mut tracker = NoteTracker::new();
    for note in [60u8, 67] {
        tracker.handle_event(NoteEvent::on(0.0, 0, note, 1.0));
    }
    let view = ViewConfig { extent_threes: 3, extent_fives: 3, ..ViewConfig::default() };
    let silent = dots_of(&view);
    let played = dots_of_with(&view, &tracker);
    assert_eq!(silent.len(), played.len(), "a chord changed how many positions are marked");
    for (a, b) in silent.iter().zip(&played) {
        assert_eq!(a.pos, b.pos, "a chord moved a dot");
        assert_eq!(a.color, b.color, "a chord tinted a dot: {b:?}");
        assert_eq!(a.strength, b.strength, "a chord lit a dot: {b:?}");
        assert_eq!(a.radius, b.radius, "a chord resized a dot: {b:?}");
    }
    // And the two nodes that ARE sounding still stand on the plain ground.
    let scene = scene_of(&tracker, &tuning, &view, &plain_frame(), 0.0);
    for dot in &scene.dots {
        assert_eq!(dot.color, scene.lattice_ground, "{dot:?}");
        assert_eq!(dot.strength, 1.0, "{dot:?}");
    }
    assert!(
        scene.nodes.iter().any(|n| n.activation > 0.0),
        "the chord has to reach the lattice for this to mean anything",
    );
}

#[test]
fn an_off_sheet_note_leaves_the_dot_field_alone() {
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
        scene.dots.len(),
        1,
        "a column one node wide has one home position, so one dot: {:?}",
        scene.dots,
    );
    assert!(scene.dots[0].pos.z.abs() < 1e-5, "and it is the home sheet's: {:?}", scene.dots[0]);
}
