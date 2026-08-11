//! The lattice structure drawn between nodes: segments, their inset and
//! colour, and the chains that link a played note to what is under it.

use crate::*;
use glam::Vec3;
use harmonigraph_core::{NoteEvent, NoteTracker, Tuning};
use super::harness::*;

#[test]
fn grid_segments_connect_neighbors_but_leave_node_gaps() {
    // A 3×3 window: 2·3 horizontal + 3·2 vertical inter-neighbor
    // segments, none along the unused sevens axis. The gap-clears-the-disc
    // half of this is a claim about ONE inset (the classic 1.05, which was
    // chosen to contain the classic 0.46 disc), so pin both here rather
    // than ride whatever the current defaults are — a smaller default gap
    // is a look choice, not a regression.
    let view = ViewConfig {
        extent_threes: 1,
        extent_fives: 1,
        extent_sevens: 0,
        grid_inset: 1.05,
        core_radius: 0.46,
        ..ViewConfig::default()
    };
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &view,
        &plain_frame(),
        0.0,
    );
    assert_eq!(scene.grid.len(), 12);
    for seg in &scene.grid {
        // Inset at both ends: shorter than the node spacing...
        let len = seg.a.distance(seg.b);
        assert!(len < view.spacing * 0.99, "segment not inset, len {len}");
        // ...and clear of every disc (visual radius ~0.9 × node_radius),
        // so the gap fully contains the circle a played note draws.
        for node in &scene.nodes {
            for p in [seg.a, seg.b] {
                assert!(
                    p.distance(node.world_pos) > scene.node_radius * 0.9,
                    "segment endpoint {p:?} inside the disc at {:?}",
                    node.world_pos
                );
            }
        }
    }

    // Panning the window keeps the grid attached to the visible nodes
    // (both are derived in centered world space).
    let panned = ViewConfig { center_threes: 3, ..view };
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &panned,
        &plain_frame(),
        0.0,
    );
    assert_eq!(scene.grid.len(), 12);
    let max_node = scene
        .nodes
        .iter()
        .map(|n| n.world_pos.length())
        .fold(0.0f32, f32::max);
    for seg in &scene.grid {
        assert!(seg.a.length() <= max_node && seg.b.length() <= max_node);
    }
}

/// A 7x7 window one sevens step deep, so sevens links are in play too.
/// Wide enough that a held C also lands on an off-sheet node — under
/// 12-TET that first happens at (2, -3, 1) — which is what lights the
/// links; a ±1 window has no such node and would show none at all.
fn grid_view() -> ViewConfig {
    ViewConfig {
        extent_threes: 3,
        extent_fives: 3,
        extent_sevens: 1,
        ..plain_view()
    }
}

fn grid_of(view: &ViewConfig) -> Vec<EdgeInstance> {
    grid_of_with(view, &NoteTracker::new())
}

fn grid_of_with(view: &ViewConfig, tracker: &NoteTracker) -> Vec<EdgeInstance> {
    scene_of(tracker, &Tuning::default(), view, &plain_frame(), 0.0).grid
}

#[test]
fn grid_inset_sets_how_far_lines_stop_short_of_a_node() {
    // The inset is the "line length" knob: at 0 a segment spans the
    // full node spacing, and raising it eats the line from both ends.
    let flush = grid_of(&ViewConfig { grid_inset: 0.0, ..grid_view() });
    let spacing = grid_view().spacing;
    for seg in &flush {
        assert!(
            (seg.a.distance(seg.b) - spacing).abs() < 1e-5,
            "inset 0 should span the whole spacing, got {}",
            seg.a.distance(seg.b)
        );
    }

    let mut lengths = vec![];
    for inset in [0.0f32, 0.5, 1.05, 2.0] {
        let grid = grid_of(&ViewConfig { grid_inset: inset, ..grid_view() });
        lengths.push(grid[0].a.distance(grid[0].b));
    }
    for pair in lengths.windows(2) {
        assert!(pair[1] < pair[0], "more inset must mean shorter lines: {lengths:?}");
    }
}

#[test]
fn an_unlit_node_carries_the_idle_grey_and_draws_nothing() {
    // An idle node has no mark of its own: the grid's gap around it is what
    // says a position is there. `color` is what a node with no voice on it
    // falls back to, and nothing draws while it holds that -- so this pins
    // the neutral rather than a look, and pins that the trail never
    // overwrites it (see the trail tests).
    let line = skin::grid_line();
    assert!(line.w < 1.0, "the lines are the faint half of the pair");
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &grid_view(),
        &plain_frame(),
        0.0,
    );
    let idle = scene
        .nodes
        .iter()
        .find(|n| n.activation == 0.0)
        .expect("nothing is playing");
    assert_eq!(idle.color, line.with_w(1.0));
    assert!(
        scene.nodes.iter().all(|n| n.activation == 0.0),
        "nothing sounds, so every node is idle",
    );
}

#[test]
fn the_grid_draws_in_the_chromes_hairline_grey() {
    // The idle structure has no color of its own and no setting: it draws in
    // the grey the panel rules ITSELF with, so the picture and the chrome
    // around it cannot drift apart. Compared against the skin's bytes rather
    // than against `grid_line`'s own output, which is what makes a re-added
    // near-copy of the hairline fail here instead of passing against itself.
    let [r, g, b] = skin::active_skin().hairline;
    let unlit = grid_of(&grid_view())
        .into_iter()
        .find(|s| s.strength > 0.0)
        .expect("the home sheet draws an idle grid");
    assert_eq!(
        unlit.color.truncate(),
        Vec3::new(f32::from(r), f32::from(g), f32::from(b)) / 255.0,
    );
    assert_eq!(unlit.strength, unlit.color.w, "alpha is the idle line opacity");
}

#[test]
fn a_sevens_link_dashes_and_an_in_sheet_line_never_does() {
    // The dash is structural, not a style, and there is no setting either
    // way: it is the whole of what tells a depth link from a line drawn
    // within one sheet, so a dashed in-plane line would say "depth" about a
    // line that has none.
    let grid = grid_of_with(&grid_view(), &sounding());
    assert!(
        grid.iter().any(|s| (s.b.z - s.a.z).abs() > 1e-5),
        "want some sevens links in the mix"
    );
    assert!(
        grid.iter().any(|s| (s.b.z - s.a.z).abs() <= 1e-5),
        "and some in-sheet lines, or the claim is half-tested"
    );
    for seg in &grid {
        let along_sevens = (seg.b.z - seg.a.z).abs() > 1e-5;
        assert_eq!(seg.dashed, along_sevens, "only depth links dash");
    }
}

#[test]
fn grid_lines_never_light_between_played_neighbors() {
    // In-plane grid lines must not brighten and take the notes' color when
    // the notes at BOTH ends sound. Drawing a chord's intervals as geometry
    // reads as noise rather than structure; the grid is purely the idle
    // structure, and the only thing that lights is a sevens-axis chain (see
    // the off-sheet test above), which is about one note's depth rather
    // than a pair.
    //
    // Just intonation and a small window so pitch classes stay unique.
    let tuning = Tuning { tolerance: harmonigraph_core::tuning::microcents(5.0), ..Tuning::just() };
    let mut tracker = NoteTracker::new();
    for note in [60u8, 67] {
        tracker.handle_event(NoteEvent::on(0.0, 0, note, 1.0));
    }
    let view = ViewConfig { extent_threes: 3, extent_fives: 3, ..ViewConfig::default() };
    let scene = scene_of(&tracker, &tuning, &view, &plain_frame(), 0.0);
    let base = skin::grid_line();
    let segment_at = |mid: Vec3| {
        scene
            .grid
            .iter()
            .find(|s| ((s.a + s.b) * 0.5).distance(mid) < 1e-4)
            .unwrap()
    };

    // C sits at the origin, G one step up the threes (world y) axis: both
    // sound, and the line between them stays exactly as faint as any other.
    let between = segment_at(Vec3::new(0.0, 0.5, 0.0));
    assert_eq!(between.strength, base.w, "two sounding ends must not light it");
    assert_eq!(between.color, base, "nor tint it");

    // Every in-plane segment is identical, played over or not.
    for s in &scene.grid {
        assert_eq!(s.strength, base.w, "{s:?}");
        assert_eq!(s.color, base, "{s:?}");
    }
}

/// A sevens link is a tether for a note with nothing under it. Once there
/// is a sounding note beneath to hang from, the chain has done its job —
/// the two are already connected, visibly, by being one site a step apart.
#[test]
fn a_chain_stops_at_the_first_sounding_note_under_it() {
    let view = ViewConfig {
        extent_threes: 0,
        extent_fives: 0,
        extent_sevens: 2,
        ..plain_view()
    };
    // 12-TET default: a sevens step is 1000¢, so (0,0,1) is MIDI 70's
    // pitch class and (0,0,2) is MIDI 68's. The home node is C.
    let chain = |notes: &[u8]| -> Vec<f32> {
        let mut tracker = NoteTracker::new();
        for &note in notes {
            tracker.handle_event(NoteEvent::on(0.0, 0, note, 1.0));
        }
        let scene =
            scene_of(&tracker, &Tuning::default(), &view, &plain_frame(), 0.0);
        // The two links of the upward column, low end first.
        let mut links: Vec<&EdgeInstance> = scene
            .grid
            .iter()
            .filter(|e| (e.b.z - e.a.z).abs() > 0.25 && e.a.z >= -0.01)
            .collect();
        links.sort_by(|a, b| a.a.z.total_cmp(&b.a.z));
        // An unlit sevens link is not merely faint, it is never shipped:
        // off-sheet lines have no idle strength, so derive_grid drops the
        // instance entirely. Presence IS the assertion.
        links.iter().map(|e| e.a.z).collect()
    };

    // Floating: only the top of the column sounds, so the whole chain
    // draws — that note has nothing else to hang from.
    let floating = chain(&[68]);
    assert_eq!(floating.len(), 2, "both links: {floating:?}");

    // Anchored one step down: the note at sheet 1 sounds too, so the link
    // between 1 and 2 is redundant and goes. The link from home up to the
    // sounding sheet-1 note stays — nothing is sounding under THAT.
    let anchored = chain(&[68, 70]);
    assert_eq!(anchored.len(), 1, "only home->1 survives: {anchored:?}");
    // Endpoints are inset from the node centers, so this is "starts on the
    // home sheet" rather than "starts at exactly zero".
    assert!(anchored[0] < 0.5, "and it is the one rising from home: {anchored:?}");

    // Anchored on the home sheet itself: nothing in the column needs a
    // tether at all, so no link is drawn.
    let grounded = chain(&[68, 60]);
    assert!(grounded.is_empty(), "{grounded:?}");
}

#[test]
fn a_lit_chain_keeps_the_lattices_own_color() {
    // The chain is structure, not a note: it says WHERE a note hangs from,
    // and the note's own color is already on the node at each end. Taking
    // the note's hue made it read as a third sounding thing strung between
    // two others.
    let view = ViewConfig {
        extent_threes: 0,
        extent_fives: 0,
        extent_sevens: 1,
        ..plain_view()
    };
    let mut tracker = NoteTracker::new();
    tracker.handle_event(NoteEvent::on(0.0, 0, 70, 1.0));
    let scene =
        scene_of(&tracker, &Tuning::default(), &view, &plain_frame(), 0.0);
    let lit: Vec<&EdgeInstance> = scene
        .grid
        .iter()
        .filter(|e| (e.b.z - e.a.z).abs() > 0.25 && e.strength > 0.5)
        .collect();
    assert!(!lit.is_empty(), "the chain has to be lit for this to mean anything");
    let base = skin::grid_line();
    for link in lit {
        assert_eq!(link.color, base, "a lit link keeps the grid color");
    }
}
