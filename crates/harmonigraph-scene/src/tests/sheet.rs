//! The home sheet and what leaves it: off-sheet shrink and blanking, the
//! knockout each sounding node clears behind it, and comma spelling.

use crate::*;
use harmonigraph_core::{NoteEvent, NoteEventKind, NoteTracker, Tuning};
use super::harness::*;

#[test]
fn home_sheet_nodes_are_flagged_for_the_blank_ring() {
    // Follows the panned window center, not sevens == 0.
    let view = ViewConfig {
        extent_threes: 0,
        extent_fives: 0,
        extent_sevens: 1,
        center_sevens: 2,
        ..ViewConfig::default()
    };
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &view,
        &FrameParams::default(),
        0.0,
    );
    for n in &scene.nodes {
        assert_eq!(n.on_home, n.lattice_pos.sevens == 2, "{:?}", n.lattice_pos);
    }
}

#[test]
fn off_sheet_grid_appears_only_where_the_music_reaches() {
    // A window two sevens layers deep above/below the center, so the
    // chain rule has an intermediate link to prove itself on.
    let view = ViewConfig {
        extent_threes: 1,
        extent_fives: 0,
        extent_sevens: 2,
        ..plain_view()
    };
    let is_link = |s: &EdgeInstance| (s.b.z - s.a.z).abs() > 0.25;
    let off_home = |s: &EdgeInstance| is_link(s) || s.a.z.abs() > 0.5;

    // Idle: only the home sheet's solid lines exist.
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &view,
        &FrameParams::default(),
        0.0,
    );
    assert!(!scene.grid.is_empty());
    assert!(scene.grid.iter().all(|s| !off_home(s) && !s.dashed && s.strength > 0.0));

    // Hold the note two sevens steps up from C (12-TET default:
    // 2 × 1000¢ → pitch class 800¢ = G#/Ab, MIDI 68). It lights node
    // (0,0,2) only, yet BOTH links of the chain down to the home
    // sheet must display, dashed, in that note's color — the nodes
    // under it are silent.
    let mut tracker = NoteTracker::new();
    tracker.handle_event(NoteEvent {
        time: 0.0,
        channel: 0,
        note: 68,
        kind: NoteEventKind::On { velocity: 1.0 },
    });
    let scene =
        scene_of(&tracker, &Tuning::default(), &view, &FrameParams::default(), 0.0);
    let column_links: Vec<&EdgeInstance> = scene
        .grid
        .iter()
        .filter(|s| is_link(s) && s.a.x.abs() < 0.01 && s.a.y.abs() < 0.01)
        .collect();
    // The two links spanning 0->1 and 1->2; nothing below the sheet.
    assert_eq!(column_links.len(), 2, "{column_links:?}");
    for link in &column_links {
        assert!(link.a.z > -0.5 && link.dashed && link.strength > 0.5, "{link:?}");
    }
    // No off-sheet IN-SHEET lines appeared: the played node's sheet
    // neighbors are silent, so only the chain and home sheet render.
    assert!(scene
        .grid
        .iter()
        .all(|s| is_link(s) || s.a.z.abs() < 0.5));
}

#[test]
fn the_mark_ring_thickness_reaches_the_scene_and_is_clamped() {
    // One thickness drives BOTH rings, so it lives on the scene rather
    // than per node; 0 is the off state, as it is for the core's radius.
    let view = ViewConfig { mark_thickness: 0.15, ..ViewConfig::default() };
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &view,
        &FrameParams::default(),
        0.0,
    );
    assert_eq!(scene.mark_thickness, 0.15);

    let off = ViewConfig { mark_thickness: 0.0, ..ViewConfig::default() };
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &off,
        &FrameParams::default(),
        0.0,
    );
    assert_eq!(scene.mark_thickness, 0.0, "0 passes through as the off state");

    let wild = ViewConfig { mark_thickness: 9.0, ..ViewConfig::default() };
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &wild,
        &FrameParams::default(),
        0.0,
    );
    assert!(scene.mark_thickness <= 0.4, "got {}", scene.mark_thickness);
}

#[test]
fn the_octave_gap_reaches_the_scene_and_is_clamped() {
    // One padding for the whole octave layer: the shader spaces the
    // sectors AND the melody/bass rings by this single number, so it has
    // to survive derive_scene rather than being a shader constant.
    let view = ViewConfig { outer_gap: 0.2, ..ViewConfig::default() };
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &view,
        &FrameParams::default(),
        0.0,
    );
    assert_eq!(scene.outer_gap, 0.2);

    // A gap wider than the band would erase every sector; the scene caps
    // it rather than handing the shader something that draws nothing.
    let wild = ViewConfig { outer_gap: 5.0, ..ViewConfig::default() };
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &wild,
        &FrameParams::default(),
        0.0,
    );
    assert!(scene.outer_gap <= 0.4, "got {}", scene.outer_gap);
}

#[test]
fn core_and_outer_geometry_are_sanitized_into_the_scene() {
    // Bars dragged into a crossed/degenerate combination: the scene
    // must still hand the shader a visible band (outer ahead of inner).
    // A radius of 0 turns the core off (passes through as 0).
    let view = ViewConfig {
        core_radius: 0.0,
        outer_inner: 0.8,
        outer_outer: 0.3,
        ..ViewConfig::default()
    };
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &view,
        &FrameParams::default(),
        0.0,
    );
    assert_eq!(scene.core_radius, 0.0, "radius 0 = core off");
    assert_eq!(scene.outer_inner, 0.8);
    assert!(scene.outer_outer > scene.outer_inner);

    // Core on: the radius passes through and solidity rides alongside,
    // both clamped to range.
    let view = ViewConfig {
        core_radius: 0.3,
        core_solidity: 0.25,
        outer_inner: 0.0,
        outer_outer: 0.5,
        ..view
    };
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &view,
        &FrameParams::default(),
        0.0,
    );
    assert_eq!(scene.core_radius, 0.3);
    assert_eq!(scene.core_solidity, 0.25);
    assert_eq!((scene.outer_inner, scene.outer_outer), (0.0, 0.5));

    // Core solidity is clamped into 0..1 before it reaches the shader.
    let view = ViewConfig { core_solidity: 4.0, ..view };
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &view,
        &FrameParams::default(),
        0.0,
    );
    assert_eq!(scene.core_solidity, 1.0);
}

#[test]
fn off_sheet_nodes_shrink_away_from_the_home_sheet_both_ways() {
    // Size says DISTANCE from the home sheet, not depth toward the eye: a
    // sheet in front shrinks exactly as much as one behind. The home sheet
    // is the ground the music is read against and stays full size.
    let view = ViewConfig {
        extent_sevens: 2,
        sevens_size: 0.5,
        ..ViewConfig::default()
    };
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &view,
        &FrameParams::default(),
        0.0,
    );
    assert_eq!(node_at(&scene, LatticePos::new(0, 0, 0)).scale, 1.0);
    for sevens in [-1, 1] {
        assert_eq!(node_at(&scene, LatticePos::new(0, 0, sevens)).scale, 0.5);
    }
    for sevens in [-2, 2] {
        assert_eq!(node_at(&scene, LatticePos::new(0, 0, sevens)).scale, 0.25);
    }
}

#[test]
fn sevens_size_never_enlarges_and_never_vanishes() {
    // The axis only ever makes off-sheet nodes SMALLER (a value above 1
    // would put the sevens layer in front of the picture it annotates), and
    // never small enough to disappear at the far extents.
    let scene_with = |size: f32| {
        let view = ViewConfig { extent_sevens: 4, sevens_size: size, ..ViewConfig::default() };
        scene_of(&NoteTracker::new(), &Tuning::default(), &view, &FrameParams::default(), 0.0)
    };
    let huge = scene_with(4.0);
    assert_eq!(node_at(&huge, LatticePos::new(0, 0, 4)).scale, 1.0, "clamped to no growth");
    let tiny = scene_with(0.0);
    assert!(
        node_at(&tiny, LatticePos::new(0, 0, 4)).scale > 0.0001,
        "the farthest sheet still draws something"
    );
}

#[test]
fn every_sounding_node_clears_what_is_behind_it_the_home_sheet_included() {
    // The knockout is not an off-sheet ornament, it is how one sheet hides
    // another — so the home sheet needs one too, or the sheets behind it
    // show straight through the gaps in a home node's body (a small soft
    // core and a thin gapped annulus cover very little) and neither sheet
    // reads as being in front. Which layer the clearing is DRAWN in differs
    // — the home sheet's goes ahead of the grid, see the renderer — but
    // that is not this layer's business.
    let view = ViewConfig { extent_sevens: 1, sevens_gutter: 0.2, ..plain_view() };
    let scene = scene_of(&held(60), &Tuning::default(), &view, &FrameParams::default(), 0.0);

    // C sounds, so every node whose pitch class is C lights — on the home
    // sheet and off it. All of them clear; the silent ones do not.
    let mut lit_home = 0;
    let mut lit_off = 0;
    for node in &scene.nodes {
        if node.activation > 0.0 {
            assert_eq!(node.gutter, 0.2, "a sounding node clears, wherever it is");
            if node.on_home {
                lit_home += 1;
            } else {
                lit_off += 1;
            }
        } else {
            assert_eq!(node.gutter, 0.0, "a silent node punches nothing");
        }
    }
    assert!(lit_home > 0 && lit_off > 0, "the case needs both kinds lit");
}

#[test]
fn a_flat_lattice_still_clears_its_grid() {
    // With the sevenths extent at 0 there is no sheet behind anything, but
    // the clearing is not only an inter-sheet device: it cuts the grid
    // lines, so a sounding node sits in a clean gap in the lattice rather
    // than on top of it. That reading is wanted at any depth, so the home
    // sheet clears on a flat lattice exactly as it does on a deep one.
    // Gating it on the extent would make the look reachable only by growing
    // depth the view doesn't want.
    let view = ViewConfig { extent_sevens: 0, sevens_gutter: 0.2, ..plain_view() };
    let scene = scene_of(&held(60), &Tuning::default(), &view, &FrameParams::default(), 0.0);
    let mut lit = 0;
    for node in &scene.nodes {
        if node.activation > 0.0 {
            assert_eq!(node.gutter, 0.2, "a sounding home node clears with no depth at all");
            lit += 1;
        } else {
            assert_eq!(node.gutter, 0.0, "a silent node punches nothing");
        }
    }
    assert!(lit > 0, "something is lit");
}

#[test]
fn a_releasing_note_keeps_its_gutters_width() {
    // `gutter` is the clearing's WIDTH, and it must not follow the
    // envelope: the shader fades the clearing's STRENGTH by the same
    // `activation` it paints the node with, and doing both would shrink
    // the hole as it faded.
    //
    // The regression this pins is the other way round, and much worse.
    // Scaling the width alone (the first cut) leaves the clearing fully
    // opaque for the entire release and merely hardens its soft edge — so
    // the hole sits there at full strength while its note fades away under
    // it, then disappears the instant the voice is pruned. It reads as a
    // pop, which is exactly what it is.
    let view = ViewConfig {
        extent_sevens: 1,
        sevens_gutter: 0.2,
        ..ViewConfig::default()
    };
    let frame = FrameParams { fade_time: 1.0, ..FrameParams::default() };
    let tuning = Tuning::default();
    let mut tracker = held(60);
    tracker.handle_event(NoteEvent {
        time: 0.0,
        channel: 0,
        note: 60,
        kind: NoteEventKind::Off,
    });

    let off_sheet = |now: f64| {
        let scene = scene_of(&tracker, &tuning, &view, &frame, now);
        *scene
            .nodes
            .iter()
            .find(|n| n.activation > 0.0 && n.lattice_pos.sevens != 0)
            .expect("a lit off-sheet node")
    };
    // A quarter and three quarters of the way through the release: the note
    // is measurably dimmer, and the clearing is exactly as wide.
    let early = off_sheet(0.25);
    let late = off_sheet(0.75);
    assert!(late.activation < early.activation, "the note really is fading");
    assert_eq!(early.gutter, 0.2);
    assert_eq!(late.gutter, 0.2, "the clearing holds its width through the fade");
}

#[test]
fn the_comma_measures_the_node_against_its_own_namesake() {
    // `note_name` walks the fifths with `threes + fives*4 - sevens*2`, so a
    // sevens step lands on the LETTER two fifths down. The comma is the
    // distance to that node — the septimal comma, 64/63, ~27 cents at just
    // intonation.
    let view = ViewConfig { extent_sevens: 1, ..ViewConfig::default() };
    let tuning = Tuning::just();
    let scene =
        scene_of(&NoteTracker::new(), &tuning, &view, &FrameParams::default(), 0.0);

    let seventh = LatticePos::new(0, 0, 1);
    let namesake = LatticePos::new(-2, 0, 0);
    // The premise: the two share a letter and an accidental, which is what
    // makes one the other's namesake. They are no longer the same NAME —
    // the septimal mark is what tells them apart, and this comma is the
    // distance that mark stands for.
    let (a, b) = (seventh.note_name(), namesake.note_name());
    assert_eq!((a.letter, a.accidental_mark()), (b.letter, b.accidental_mark()));
    assert_ne!(a.to_string(), b.to_string());
    let comma = node_at(&scene, seventh).comma;
    assert!(
        (comma - -27.26).abs() < 0.05,
        "7/4 sits a septimal comma below 16/9, got {comma}"
    );
    // The other direction is the same distance the other way, and the home
    // sheet has no namesake to measure against.
    let below = node_at(&scene, LatticePos::new(0, 0, -1)).comma;
    assert!((below - 27.26).abs() < 0.05, "got {below}");
    assert_eq!(node_at(&scene, LatticePos::ORIGIN).comma, 0.0);
}

#[test]
fn the_comma_takes_the_short_way_round_the_octave() {
    // Pitch classes wrap, so a raw subtraction can come out an octave off
    // and report a 1173-cent "comma". Two sevens steps land far enough
    // round the circle to catch it.
    let view = ViewConfig { extent_sevens: 3, ..ViewConfig::default() };
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::just(),
        &view,
        &FrameParams::default(),
        0.0,
    );
    for sevens in [-3, -2, -1, 1, 2, 3] {
        let comma = node_at(&scene, LatticePos::new(0, 0, sevens)).comma;
        assert!(
            comma.abs() <= 600.0,
            "sevens {sevens}: comma {comma} is the long way round"
        );
    }
}

#[test]
fn the_knockout_clears_to_the_ground_not_to_black() {
    // The gutter has no color of its own, so a premultiplied layer would
    // knock out to BLACK — and this skin's panel is several shades lighter
    // than black, which is exactly what made the cleared disc read as a
    // dark plate sitting on the picture instead of a hole through it. The
    // scene therefore carries the ground, and it must be the panel the
    // dock paints under every pane, not zero.
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &ViewConfig::default(),
        &FrameParams::default(),
        0.0,
    );
    let panel = crate::skin::active_skin().panel;
    assert_eq!(scene.background, crate::skin::ground_color((panel[0], panel[1], panel[2])));
    assert!(scene.background.truncate().length() > 0.0, "not black");
    assert_eq!(scene.background.w, 1.0, "opaque, or it would not cover");
}

#[test]
fn ground_color_keeps_srgb_bytes_as_they_are() {
    // A straight divide by 255, NOT a gamma decode: every color the shader
    // handles is sRGB-encoded 0..1 (the offscreen target is a plain Unorm
    // format), so decoding here would clear the gutter to a ground far
    // darker than the pane it is supposed to disappear into.
    let c = crate::skin::ground_color((24, 25, 29));
    assert!((c.x - 24.0 / 255.0).abs() < 1e-6);
    assert!((c.y - 25.0 / 255.0).abs() < 1e-6);
    assert!((c.z - 29.0 / 255.0).abs() < 1e-6);
}
