//! Unit tests for the scene layer.

use super::*;
use crate::derive::{depth_scale, held_extremes, DEPTH_SCALE_RANGE};
use glam::{Vec2, Vec3, Vec4};
use lattice_core::{NoteEvent, NoteEventKind, NoteTracker, PitchClass, Tuning};

fn scene_of(
    tracker: &NoteTracker,
    tuning: &Tuning,
    view: &ViewConfig,
    frame: &FrameParams,
    now: f64,
) -> Scene {
    derive_scene(tracker, tuning, view, frame, Camera::default(), None, now)
}

fn origin_node(scene: &Scene) -> &NodeInstance {
    scene
        .nodes
        .iter()
        .find(|n| n.lattice_pos == LatticePos::ORIGIN)
        .unwrap()
}

#[test]
fn pitch_colored_channels_vary_with_pitch() {
    let low = channel_color(9, 24.0, 24.0, 108.0);
    let high = channel_color(9, 108.0, 24.0, 108.0);
    assert_ne!(low, high);
    // Brightest pitch should be, well, brighter.
    assert!(high.truncate().length() > low.truncate().length());
}

#[test]
fn pitch_lut_lut_reproduces_the_pitch_gradient() {
    // The octave glyphs tint each slot by sampling `pitch_ramp_lut`
    // the way the shader does (linear interp across PITCH_LUT_N entries).
    // Reconstructing that here must land on the disc's gradient color for
    // the same pitch, so a dot is the color of the disc its pitch lights.
    let lut = pitch_ramp_lut();
    let (dark, bright) = (24.0f32, 108.0f32);
    for pitch in [24.0f32, 36.0, 54.0, 60.0, 72.0, 96.0, 108.0] {
        let t = ((pitch - dark) / (bright - dark)).clamp(0.0, 1.0);
        let f = t * (PITCH_LUT_N - 1) as f32;
        let i0 = f.floor() as usize;
        let i1 = (i0 + 1).min(PITCH_LUT_N - 1);
        let lut_color = lut[i0].lerp(lut[i1], f - f.floor());
        // Same pitch through the disc path (channel 9 is pitch-gradient).
        let disc = channel_color(9, pitch, dark, bright);
        assert!(
            (lut_color - disc).truncate().length() < 0.05,
            "pitch {pitch}: lut {lut_color:?} vs disc {disc:?}"
        );
    }
}

#[test]
fn octaves_fade_independently() {
    // Hold C4, tap-and-release C5: the octave-5 indicator must decay on
    // its own envelope even though the node stays fully active.
    let mut tracker = NoteTracker::new();
    for (note, kind) in [
        (60, NoteEventKind::On { velocity: 1.0 }), // C4 held
        (72, NoteEventKind::On { velocity: 1.0 }), // C5 tapped...
    ] {
        tracker.handle_event(NoteEvent { time: 0.0, channel: 0, note, kind });
    }
    tracker.handle_event(NoteEvent {
        time: 0.1,
        channel: 0,
        note: 72,
        kind: NoteEventKind::Off, // ...and released
    });

    // Half a fade_time after the release.
    let frame = FrameParams { fade_time: 1.0, ..FrameParams::default() };
    let scene =
        scene_of(&tracker, &Tuning::default(), &ViewConfig::default(), &frame, 0.6);
    let origin = origin_node(&scene);
    assert_eq!(origin.activation, 1.0, "node stays lit by the held C4");
    assert_eq!(origin.octaves[4], 1.0, "held octave at full");
    assert!(
        origin.octaves[5] > 0.0 && origin.octaves[5] < 0.75,
        "released octave mid-fade, got {}",
        origin.octaves[5]
    );
}

#[test]
fn one_fade_time_carries_every_layer_of_the_node() {
    // The core, the octave glyphs, and the melody/bass marks all ride the
    // single Fade param: release a two-note chord and half a fade later
    // every layer must be half-way down together, none of them already
    // dark and none still at full.
    let mut tracker = NoteTracker::new();
    for note in [60u8, 67] {
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
    }
    for note in [60u8, 67] {
        tracker.handle_event(NoteEvent { time: 0.0, channel: 0, note, kind: NoteEventKind::Off });
    }
    let frame = FrameParams { fade_time: 2.0, ..FrameParams::default() };
    tracker.prune(1.0, frame.fade_time);
    let view = ViewConfig {
        highlight_extremes: HighlightExtremes::Both,
        ..ViewConfig::default()
    };
    let scene = scene_of(&tracker, &Tuning::default(), &view, &frame, 1.0);

    let half = |what: &str, v: f32| {
        assert!((v - 0.5).abs() < 1e-5, "{what} should be half-faded, got {v}");
    };
    // C4 is the bass and sits on the origin node.
    let origin = origin_node(&scene);
    half("the core", origin.activation);
    // The mark rides its octave slot's envelope, so a half-faded glyph is a
    // half-faded mark; what matters here is that the slot is still MARKED
    // rather than having snapped off at release.
    half("the octave glyph", origin.octaves[4]);
    assert_eq!(origin.bass_slots, 1 << 4, "the released bass keeps its mark");
    // G4 is the melody, one fifth up the lattice.
    let melody = scene
        .nodes
        .iter()
        .find(|n| n.melody_slots != 0)
        .expect("the released melody keeps its mark while it fades");
    half("the melody's octave glyph", melody.octaves[4]);
}

#[test]
fn seed_derives_from_the_note_on_time() {
    let mut tracker = NoteTracker::new();
    tracker.handle_event(NoteEvent {
        time: 10.0,
        channel: 0,
        note: 60,
        kind: NoteEventKind::On { velocity: 1.0 },
    });
    // Steady seeds each node from its note-on time; field styles seed
    // from position instead, so pin the style this test is about.
    let view = ViewConfig { node_style: NodeStyle::Steady, ..ViewConfig::default() };
    let scene = scene_of(
        &tracker,
        &Tuning::default(),
        &view,
        &FrameParams::default(),
        12.5,
    );
    let origin = origin_node(&scene);
    assert!((origin.seed - 10.0).abs() < 1e-6);
    // Idle nodes carry neutral animation inputs.
    let idle = scene
        .nodes
        .iter()
        .find(|n| n.lattice_pos == LatticePos::new(1, 1, 0))
        .unwrap();
    assert_eq!(idle.seed, 0.0);
}

#[test]
fn window_center_pans_which_nodes_display() {
    let view = ViewConfig {
        center_threes: 5,
        extent_threes: 1,
        extent_fives: 0,
        extent_sevens: 0,
        ..ViewConfig::default()
    };
    let positions: Vec<_> = view.visible_positions().collect();
    assert_eq!(
        positions,
        vec![
            LatticePos::new(4, 0, 0),
            LatticePos::new(5, 0, 0),
            LatticePos::new(6, 0, 0)
        ]
    );

    // The center node renders at the world origin.
    let tracker = NoteTracker::new();
    let scene =
        scene_of(&tracker, &Tuning::default(), &view, &FrameParams::default(), 0.0);
    let center_node = scene
        .nodes
        .iter()
        .find(|n| n.lattice_pos == LatticePos::new(5, 0, 0))
        .unwrap();
    assert_eq!(center_node.world_pos, Vec3::ZERO);
}

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
        &FrameParams::default(),
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
        &FrameParams::default(),
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
        ..ViewConfig::default()
    }
}

fn grid_of(view: &ViewConfig) -> Vec<EdgeInstance> {
    grid_of_with(view, &NoteTracker::new())
}

fn grid_of_with(view: &ViewConfig, tracker: &NoteTracker) -> Vec<EdgeInstance> {
    scene_of(tracker, &Tuning::default(), view, &FrameParams::default(), 0.0).grid
}

/// A held note, so the off-sheet sevens links light up and appear in
/// the grid at all (idle ones are skipped as fully invisible).
fn sounding() -> NoteTracker {
    let mut tracker = NoteTracker::new();
    tracker.handle_event(NoteEvent {
        time: 0.0,
        channel: 0,
        note: 60,
        kind: NoteEventKind::On { velocity: 1.0 },
    });
    tracker
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
fn the_grid_color_drives_the_idle_nodes_too() {
    // Grid lines and idle markers are one visual layer -- the idle
    // structure -- so they share a color. The markers take only the
    // RGB: the grid's alpha is the LINE opacity, and letting it dim
    // the markers would dissolve them whenever the lines are faint.
    let tinted = [0.9f32, 0.1, 0.4, 0.25];
    let view = ViewConfig { grid_color: tinted, ..grid_view() };
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &view,
        &FrameParams::default(),
        0.0,
    );
    assert_eq!(scene.node_idle, Vec4::new(0.9, 0.1, 0.4, 1.0));
    let idle = scene
        .nodes
        .iter()
        .find(|n| n.activation == 0.0)
        .expect("nothing is playing");
    assert_eq!(idle.color, Vec4::new(0.9, 0.1, 0.4, 1.0));
}

#[test]
fn grid_color_and_dashes_come_from_the_view() {
    // The color (and its alpha, the idle line opacity) is a view
    // setting now; it used to be readable only from the skin.
    let tinted = [0.9f32, 0.1, 0.4, 0.25];
    let grid = grid_of(&ViewConfig { grid_color: tinted, ..grid_view() });
    let unlit = grid
        .iter()
        .find(|s| s.strength > 0.0)
        .expect("the home sheet draws an idle grid");
    assert_eq!(unlit.color, Vec4::from_array(tinted));
    assert_eq!(unlit.strength, tinted[3], "alpha is the idle line opacity");

    // Dashes: off by default for in-plane lines, on for sevens links
    // either way (that dash marks a depth link, it isn't a style).
    let tracker = sounding();
    let plain = grid_of_with(&grid_view(), &tracker);
    let dashed =
        grid_of_with(&ViewConfig { grid_dashed: true, ..grid_view() }, &tracker);
    assert!(
        plain.iter().any(|s| (s.b.z - s.a.z).abs() > 1e-5),
        "want some sevens links in the mix"
    );
    for (before, after) in plain.iter().zip(&dashed) {
        let along_sevens = (before.b.z - before.a.z).abs() > 1e-5;
        assert_eq!(before.dashed, along_sevens, "only links dash by default");
        assert!(after.dashed, "every line dashes when the style is on");
    }
}

/// Play `notes` on channel 0 and derive a scene marking both extremes.
fn marked_scene(notes: &[u8], which: HighlightExtremes) -> Scene {
    let mut tracker = NoteTracker::new();
    for &note in notes {
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
    }
    let view = ViewConfig { highlight_extremes: which, ..ViewConfig::default() };
    scene_of(&tracker, &Tuning::default(), &view, &FrameParams::default(), 0.0)
}

/// Union of a mask across every node, and the nodes carrying it.
fn marked_slots(scene: &Scene, melody: bool) -> (u32, usize) {
    let mut bits = 0u32;
    let mut nodes = 0usize;
    for n in &scene.nodes {
        let m = if melody { n.melody_slots } else { n.bass_slots };
        if m != 0 {
            bits |= m;
            nodes += 1;
        }
    }
    (bits, nodes)
}

#[test]
fn melody_and_bass_mark_the_outer_held_notes() {
    // C4/E4/G4: the melody is G4 (octave slot 4), the bass C4 (slot 4
    // too -- same MIDI octave, but different nodes/pitch classes).
    let scene = marked_scene(&[60, 64, 67], HighlightExtremes::Both);
    let (melody_bits, melody_nodes) = marked_slots(&scene, true);
    let (bass_bits, bass_nodes) = marked_slots(&scene, false);
    assert_eq!(melody_bits, 1 << 4, "G4 sounds in MIDI octave 4");
    assert_eq!(bass_bits, 1 << 4, "C4 too");
    assert!(melody_nodes > 0 && bass_nodes > 0);

    // The marks land on the nodes those notes actually light, and the
    // middle note (E4) is marked as neither.
    let tuning = Tuning::default();
    for n in &scene.nodes {
        let pc = tuning.pitch_class(n.lattice_pos);
        if n.melody_slots != 0 {
            assert!(tuning.matches(PitchClass::from_midi_note(67), pc), "melody is G");
        }
        if n.bass_slots != 0 {
            assert!(tuning.matches(PitchClass::from_midi_note(60), pc), "bass is C");
        }
    }

    // Asking for one end leaves the other unmarked.
    let melody_only = marked_scene(&[60, 64, 67], HighlightExtremes::Melody);
    assert_eq!(marked_slots(&melody_only, true).0, 1 << 4);
    assert_eq!(marked_slots(&melody_only, false).0, 0, "bass not asked for");
    let off = marked_scene(&[60, 64, 67], HighlightExtremes::Off);
    assert_eq!(marked_slots(&off, true).0, 0);
    assert_eq!(marked_slots(&off, false).0, 0);
}

#[test]
fn a_lone_held_note_is_marked_as_both_ends() {
    // One note is at once the highest and the lowest held, and is marked
    // as both. The shader splits such a mark between the two colors (see
    // mark_paint) rather than blanking it, which is what it used to do --
    // an outline that vanished exactly when two things were true at once.
    let scene = marked_scene(&[60], HighlightExtremes::Both);
    let mut seen = false;
    for n in &scene.nodes {
        assert_eq!(
            n.melody_slots, n.bass_slots,
            "a lone note must claim identical slots at both ends"
        );
        seen |= n.melody_slots != 0;
    }
    assert!(seen, "the note should have been marked somewhere");
}

#[test]
fn a_chord_inside_one_pitch_class_separates_on_the_octave_layer() {
    // C2 and C4: one pitch class, so both land on the SAME node and the
    // core can't say which is which -- but they sound in different
    // octave slots, which is what keeps them tellable apart.
    let scene = marked_scene(&[48, 72], HighlightExtremes::Both);
    let marked: Vec<_> = scene
        .nodes
        .iter()
        .filter(|n| n.melody_slots != 0 || n.bass_slots != 0)
        .collect();
    assert!(!marked.is_empty(), "C should be marked");
    for n in &marked {
        // MIDI octave = note/12 - 1, so C4 (72) is slot 5, C2 (48) slot 3.
        assert_eq!(n.melody_slots, 1 << 5, "the high C is the melody");
        assert_eq!(n.bass_slots, 1 << 3, "the low C is the bass");
        // No slot claimed by both, so the octave layer marks each end
        // rather than suppressing them the way the core has to.
        assert_eq!(n.melody_slots & n.bass_slots, 0);
    }
}

#[test]
fn a_released_mark_fades_out_while_a_held_note_keeps_its_node_lit() {
    // The motivating case for tracking mark levels apart from the node's
    // activation. C4 and C5 share a pitch class, so they light ONE node.
    // Release the top one: the node stays fully lit by the held C4, but
    // the melody mark it was wearing has to fade out on its own.
    let mut tracker = NoteTracker::new();
    for note in [60u8, 72] {
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
    }
    tracker.handle_event(NoteEvent { time: 0.0, channel: 0, note: 72, kind: NoteEventKind::Off });
    let frame = FrameParams { fade_time: 2.0, ..FrameParams::default() };
    let view = ViewConfig {
        highlight_extremes: HighlightExtremes::Both,
        ..ViewConfig::default()
    };
    let scene = scene_of(&tracker, &Tuning::default(), &view, &frame, 1.0);
    let origin = origin_node(&scene);
    assert_eq!(origin.activation, 1.0, "the held C4 keeps the node lit");
    // The released C5 wears a fading melody mark on ITS octave slot while
    // C4, now the top held note, takes the live one — the two crossfade
    // rather than the mark jumping. C4 is also the bass (it is the only
    // note held), so it claims both ends and the shader splits its glyph.
    assert_eq!(origin.melody_slots, (1 << 5) | (1 << 4), "the two melodies crossfade");
    assert_eq!(origin.bass_slots, 1 << 4, "only the held C4 is the bass");
    // The octave marks fade with their own slots, which is what separates
    // the fading C5 from the held C4 here.
    assert!(
        (origin.octaves[5] - 0.5).abs() < 1e-5,
        "the released C5's octave is half-faded, got {}",
        origin.octaves[5]
    );
    assert_eq!(origin.octaves[4], 1.0, "the held C4's octave is at full");
}

#[test]
fn held_extremes_never_names_a_released_voice() {
    // A released voice keeps whatever mark it was wearing (above), but it
    // is out of the running for the LIVE ends: letting it stay "the
    // melody" would steal that from the note that actually replaced it.
    let mut tracker = NoteTracker::new();
    for note in [60u8, 67] {
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
    }
    // Release the top note; C is now both the highest and lowest held.
    tracker.handle_event(NoteEvent {
        time: 0.1,
        channel: 0,
        note: 67,
        kind: NoteEventKind::Off,
    });
    let (melody, bass) = held_extremes(&tracker, HighlightExtremes::Both);
    assert_eq!(melody, Some((0, 60)), "the released G must not stay the melody");
    assert_eq!(bass, Some((0, 60)));

    // Nothing held at all: nothing to mark.
    tracker.handle_event(NoteEvent {
        time: 0.2,
        channel: 0,
        note: 60,
        kind: NoteEventKind::Off,
    });
    assert_eq!(held_extremes(&tracker, HighlightExtremes::Both), (None, None));
}

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
        ..ViewConfig::default()
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
fn the_mark_style_reaches_the_scene() {
    // Which family the marks are drawn in is a whole-scene choice, not a
    // per-node one: every node's sectors are reshaped the same way.
    for style in [
        MarkStyle::Rings,
        MarkStyle::Extend,
        MarkStyle::Cut,
        MarkStyle::Point,
        MarkStyle::Notch,
        MarkStyle::Cap,
    ] {
        let view = ViewConfig { mark_style: style, ..ViewConfig::default() };
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &view,
            &FrameParams::default(),
            0.0,
        );
        assert_eq!(scene.mark_style, style);
    }
    // Every style has its own shader branch, so no two may share an index.
    let mut seen = std::collections::HashSet::new();
    for style in [
        MarkStyle::Rings,
        MarkStyle::Extend,
        MarkStyle::Cut,
        MarkStyle::Point,
        MarkStyle::Notch,
        MarkStyle::Cap,
    ] {
        assert!(seen.insert(style.shader_index()), "{style:?} reuses an index");
    }
    assert!(MarkStyle::Rings.is_rings());
    assert!(!MarkStyle::Point.is_rings(), "the sector styles draw no rings");
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
        outer_style: OuterStyle::Slices,
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
    assert_eq!(scene.outer_style, OuterStyle::Slices);
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

    // Both solidities are clamped into 0..1 before they reach the shader.
    let view = ViewConfig { core_solidity: 4.0, outer_solidity: -1.0, ..view };
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &view,
        &FrameParams::default(),
        0.0,
    );
    assert_eq!(scene.core_solidity, 1.0);
    assert_eq!(scene.outer_solidity, 0.0);
}

#[test]
fn legacy_core_modes_fold_onto_radius_and_solidity() {
    // The pre-radius-off modes collapse onto core_radius (0 = off) +
    // solidity: None -> radius 0, the solid Orb -> solidity 1, the
    // glow-only mode -> solidity 0.
    let mut none = ViewConfig { core_style: CoreStyle::None, ..ViewConfig::default() };
    none.migrate_legacy();
    assert_eq!(none.core_radius, 0.0, "None folds to radius 0 (off)");

    let mut orb = ViewConfig { core_style: CoreStyle::Orb, ..ViewConfig::default() };
    orb.migrate_legacy();
    assert_eq!(orb.core_solidity, 1.0);
    assert!(orb.core_radius > 0.0, "orb stays on");

    let mut glow = ViewConfig { core_style: CoreStyle::Glow, ..ViewConfig::default() };
    glow.migrate_legacy();
    assert_eq!(glow.core_solidity, 0.0);
    assert!(glow.core_radius > 0.0, "glow stays on");
}

#[test]
fn legacy_node_body_folds_into_core_and_outer() {
    // Blobs from the one-build NodeBody experiment: an octave-only body
    // becomes the core glow (solidity 0, the old core-off under-glow) +
    // the outer layer with the backdrop on. Each body once had its own
    // matching glyph shape, but only slices survives, so all three land
    // there; what still has to hold is that the blob PARSES and the core
    // drops to the glow end. Disc leaves defaults alone.
    for (body, outer) in [
        (LegacyNodeBody::Slices, OuterStyle::Slices),
        (LegacyNodeBody::Rings, OuterStyle::Slices),
        (LegacyNodeBody::Beads, OuterStyle::Slices),
    ] {
        let mut view = ViewConfig { node_body: body, ..ViewConfig::default() };
        view.migrate_legacy();
        assert_eq!(view.core_solidity, 0.0, "{body:?}");
        assert!(view.core_radius > 0.0, "{body:?} still on");
        assert_eq!(view.outer_style, outer, "{body:?}");
        assert_eq!(view.outer_backdrop, 1.0, "{body:?}");
        assert_eq!(view.node_body, LegacyNodeBody::Disc, "shim consumed");
    }

    let mut view = ViewConfig { node_body: LegacyNodeBody::Disc, ..ViewConfig::default() };
    view.migrate_legacy();
    assert_eq!(view.core_solidity, ViewConfig::default().core_solidity);
    assert_eq!(view.outer_style, ViewConfig::default().outer_style);
}

#[test]
fn depth_scale_exaggerates_proximity() {
    // Neutral at the focus distance, monotonic on either side, clamped
    // at the extremes.
    assert!((depth_scale(12.0, 12.0) - 1.0).abs() < 1e-6);
    assert!(depth_scale(6.0, 12.0) > 1.0);
    assert!(depth_scale(24.0, 12.0) < 1.0);
    assert_eq!(depth_scale(0.001, 12.0), DEPTH_SCALE_RANGE.1);
    assert_eq!(depth_scale(1e6, 12.0), DEPTH_SCALE_RANGE.0);

    // And the scene wires it in: the node nearest the eye renders
    // larger than the farthest one.
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &ViewConfig::default(),
        &FrameParams::default(),
        0.0,
    );
    let eye = scene.camera.eye();
    let dist = |n: &&NodeInstance| n.world_pos.distance(eye);
    let near = scene.nodes.iter().min_by(|a, b| dist(a).total_cmp(&dist(b))).unwrap();
    let far = scene.nodes.iter().max_by(|a, b| dist(a).total_cmp(&dist(b))).unwrap();
    assert!(
        near.scale > far.scale,
        "near {} vs far {}",
        near.scale,
        far.scale
    );
}

#[test]
fn grid_lines_never_light_between_played_neighbors() {
    // In-plane grid lines used to brighten and take the notes' color when
    // the notes at BOTH ends sounded, drawing a chord's intervals as
    // geometry. It read as noise rather than structure and is gone; the
    // grid is now purely the idle structure, and the only thing that still
    // lights is a sevens-axis chain (see the off-sheet test above), which
    // is about one note's depth rather than a pair.
    //
    // Just intonation and a small window so pitch classes stay unique.
    let tuning = Tuning { tolerance: lattice_core::tuning::microcents(5.0), ..Tuning::just() };
    let mut tracker = NoteTracker::new();
    for note in [60u8, 67] {
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
    }
    let view = ViewConfig { extent_threes: 3, extent_fives: 3, ..ViewConfig::default() };
    let scene = scene_of(&tracker, &tuning, &view, &FrameParams::default(), 0.0);
    let base = Vec4::from_array(view.grid_color);
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

#[test]
fn channel_14_voices_render_outlined() {
    let mut tracker = NoteTracker::new();
    tracker.handle_event(NoteEvent {
        time: 0.0,
        channel: 14,
        note: 60,
        kind: NoteEventKind::On { velocity: 1.0 },
    });
    let scene = scene_of(
        &tracker,
        &Tuning::default(),
        &ViewConfig::default(),
        &FrameParams::default(),
        0.0,
    );
    assert!(origin_node(&scene).outlined);
}

#[test]
fn held_note_lights_matching_nodes() {
    let mut tracker = NoteTracker::new();
    tracker.handle_event(NoteEvent {
        time: 0.0,
        channel: 0,
        note: 60, // C4: pitch class 0, octave 4
        kind: NoteEventKind::On { velocity: 1.0 },
    });
    let tuning = Tuning::default(); // 12-TET: origin node matches C exactly
    // Sampled after OCTAVE_ATTACK_TIME: the octave indicator eases in,
    // so at the note-on instant itself it is still at zero.
    let scene = scene_of(
        &tracker,
        &tuning,
        &ViewConfig::default(),
        &FrameParams::default(),
        0.5,
    );
    let origin = origin_node(&scene);
    assert_eq!(origin.activation, 1.0);
    assert_eq!(origin.octaves[4], 1.0);
}

#[test]
fn camera_target_projects_to_viewport_center() {
    let camera = Camera::default();
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &ViewConfig::default(),
        &FrameParams::default(),
        0.0,
    );
    let viewport = Vec2::new(800.0, 600.0);
    let p = scene.project(viewport, camera.target).unwrap();
    assert!((p.x - 400.0).abs() < 0.5, "x = {}", p.x);
    assert!((p.y - 300.0).abs() < 0.5, "y = {}", p.y);
}

#[test]
fn points_behind_the_camera_do_not_project() {
    for projection in [
        Projection::Perspective,
        Projection::Orthographic,
        Projection::Cabinet,
    ] {
        let camera = Camera { projection, ..Camera::default() };
        let mut scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &ViewConfig::default(),
            &FrameParams::default(),
            0.0,
        );
        scene.camera = camera;
        // Continue from the target through the eye and beyond it.
        let behind = camera.eye() + (camera.eye() - camera.target);
        assert_eq!(
            scene.project(Vec2::new(800.0, 600.0), behind),
            None,
            "{projection:?}"
        );
    }
}

#[test]
fn cabinet_faces_the_sheet_and_shears_sevens_uniformly() {
    let viewport = Vec2::new(800.0, 600.0);
    // Orbit angles are ignored: cabinet always faces the sheet. Pin the
    // shear scale to 0.5 so the "half scale" checks below hold whatever
    // the default is.
    let camera = Camera {
        projection: Projection::Cabinet,
        yaw: 1.0,
        pitch: -0.7,
        cabinet_scale: 0.5,
        ..Camera::default()
    };
    assert_eq!(camera.eye(), Vec3::new(0.0, 0.0, camera.distance));

    let mut s = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &ViewConfig::default(),
        &FrameParams::default(),
        0.0,
    );
    s.camera = camera;
    let px = |w: Vec3| s.project(viewport, w).unwrap();

    // Target centered; front-plane steps map to pure screen axes
    // (the sheet renders undistorted).
    let origin = px(Vec3::ZERO);
    assert!((origin - Vec2::new(400.0, 300.0)).length() < 0.5, "{origin:?}");
    let dx = px(Vec3::X) - origin;
    assert!(dx.x > 1.0 && dx.y.abs() < 1e-3, "{dx:?}");
    let dy = px(Vec3::Y) - origin;
    assert!(dy.y < -1.0 && dy.x.abs() < 1e-3, "{dy:?}"); // screen y is down

    // A +sevens step (toward the viewer) is the same up-right arrow
    // anywhere on the sheet, at half scale split evenly over x/y.
    let dz = px(Vec3::Z) - origin;
    let dz_elsewhere = px(Vec3::new(3.0, -2.0, 1.0)) - px(Vec3::new(3.0, -2.0, 0.0));
    assert!(dz.distance(dz_elsewhere) < 1e-3, "{dz:?} vs {dz_elsewhere:?}");
    let k = 0.5 * std::f32::consts::FRAC_1_SQRT_2;
    assert!((dz.x - dx.x * k).abs() < 0.1, "{dz:?} vs {dx:?}");
    assert!((dz.y - dy.y * k).abs() < 0.1, "{dz:?} vs {dy:?}");

    // The knobs steer the arrow: angle 0 at full (cavalier) scale
    // shears purely horizontally, one front-plane step long.
    s.camera.cabinet_angle = 0.0;
    s.camera.cabinet_scale = 1.0;
    let dz = s.project(viewport, Vec3::Z).unwrap() - s.project(viewport, Vec3::ZERO).unwrap();
    assert!((dz.x - dx.x).abs() < 0.1 && dz.y.abs() < 1e-3, "{dz:?} vs {dx:?}");
}

#[test]
fn orthographic_matches_perspective_at_the_focus_plane_and_is_uniform() {
    let viewport = Vec2::new(800.0, 600.0);
    let perspective = Camera { projection: Projection::Perspective, ..Camera::default() };
    let ortho = Camera { projection: Projection::Orthographic, ..perspective };
    let mut s = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &ViewConfig::default(),
        &FrameParams::default(),
        0.0,
    );

    // The target projects to the viewport center in both projections.
    s.camera = ortho;
    let p = s.project(viewport, ortho.target).unwrap();
    assert!((p.x - 400.0).abs() < 0.5 && (p.y - 300.0).abs() < 0.5, "{p:?}");

    // Framing matches at the focus plane: a point one unit up (in view
    // space) from the target lands on the same pixel either way.
    let (_, up) = perspective.right_up();
    let in_plane = perspective.target + up;
    let ortho_px = s.project(viewport, in_plane).unwrap();
    s.camera = perspective;
    let persp_px = s.project(viewport, in_plane).unwrap();
    assert!(ortho_px.distance(persp_px) < 0.5, "{ortho_px:?} vs {persp_px:?}");

    // The property the projection exists for: equal world offsets give
    // equal pixel offsets at ANY depth. Step one unit right at the
    // focus plane and again two units toward the eye; perspective
    // renders the nearer step longer, orthographic identically.
    s.camera = ortho;
    let (right, _) = ortho.right_up();
    let toward_eye = (ortho.eye() - ortho.target).normalize() * 2.0;
    let d_focus = s.project(viewport, ortho.target + right).unwrap()
        - s.project(viewport, ortho.target).unwrap();
    let d_near = s.project(viewport, ortho.target + toward_eye + right).unwrap()
        - s.project(viewport, ortho.target + toward_eye).unwrap();
    assert!(d_focus.distance(d_near) < 1e-3, "{d_focus:?} vs {d_near:?}");
}

#[test]
fn pick_selects_the_node_nearest_the_pointer() {
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &ViewConfig::default(),
        &FrameParams::default(),
        0.0,
    );
    let viewport = Vec2::new(800.0, 600.0);
    // Pointer exactly on the projected origin node must pick it, not a
    // neighbor; a pointer far outside every node picks nothing.
    let origin_px = scene.project(viewport, Vec3::ZERO).unwrap();
    assert_eq!(scene.pick(viewport, origin_px, 24.0), Some(LatticePos::ORIGIN));
    assert_eq!(scene.pick(viewport, Vec2::new(-500.0, -500.0), 24.0), None);
}

#[test]
fn idle_off_sheet_nodes_are_not_pickable() {
    // An idle node off the home sheet draws nothing, so hovering where
    // it would be must not hand back its pitch. Sounding makes it
    // visible, and pickable again. Needs a sevens extent: the default view
    // is the home sheet alone, which has no off-sheet node to hover.
    let view = ViewConfig { extent_sevens: 1, ..ViewConfig::default() };
    let tuning = Tuning::default();
    let viewport = Vec2::new(800.0, 600.0);

    let idle = scene_of(
        &NoteTracker::new(),
        &tuning,
        &view,
        &FrameParams::default(),
        0.0,
    );
    let off = *idle
        .nodes
        .iter()
        .find(|n| !n.on_home)
        .expect("default view spans more than the home sheet");
    assert_eq!(off.activation, 0.0);
    assert!(!off.is_visible());
    let off_px = idle.project(viewport, off.world_pos).unwrap();
    assert_ne!(
        idle.pick(viewport, off_px, 24.0),
        Some(off.lattice_pos),
        "idle off-sheet node should not be pickable"
    );

    // Same position, now sounding: play a note carrying its pitch class.
    let pc = tuning.pitch_class(off.lattice_pos);
    let note = (60u8..72)
        .find(|&n| tuning.matches(pc, PitchClass::from_midi_note(n)))
        .expect("some MIDI note lands on this node under 12-TET");
    let mut tracker = NoteTracker::new();
    tracker.handle_event(NoteEvent {
        time: 0.0,
        channel: 0,
        note,
        kind: NoteEventKind::On { velocity: 1.0 },
    });
    let lit = scene_of(&tracker, &tuning, &view, &FrameParams::default(), 0.0);
    let lit_off = lit
        .nodes
        .iter()
        .find(|n| n.lattice_pos == off.lattice_pos)
        .unwrap();
    assert!(lit_off.activation > 0.0, "the note should light this node");
    assert!(lit_off.is_visible());
    assert_eq!(
        lit.pick(viewport, off_px, 24.0),
        Some(off.lattice_pos),
        "sounding off-sheet node should be pickable again"
    );
}

#[test]
fn camera_right_up_is_orthonormal_to_the_view() {
    let camera = Camera::default();
    let (right, up) = camera.right_up();
    assert!((right.length() - 1.0).abs() < 1e-5);
    assert!((up.length() - 1.0).abs() < 1e-5);
    assert!(right.dot(up).abs() < 1e-5);
    let view_dir = (camera.target - camera.eye()).normalize();
    assert!(right.dot(view_dir).abs() < 1e-5);
    assert!(up.dot(view_dir).abs() < 1e-5);
}

#[test]
fn camera_input_respects_clamps() {
    let mut camera = Camera::default();
    camera.orbit(Vec2::new(0.0, 10_000.0));
    assert_eq!(camera.pitch, Camera::PITCH_LIMIT);
    camera.orbit(Vec2::new(0.0, -100_000.0));
    assert_eq!(camera.pitch, -Camera::PITCH_LIMIT);
    camera.zoom(1e6);
    assert_eq!(camera.distance, Camera::MIN_DISTANCE);
    camera.zoom(-1e9);
    assert_eq!(camera.distance, Camera::MAX_DISTANCE);
    // Panning moves the target in the view plane, never toward the eye.
    let before = camera.eye() - camera.target;
    camera.pan(Vec2::new(40.0, -25.0));
    let after = camera.eye() - camera.target;
    assert!((before - after).length() < 1e-4);
}

#[test]
fn zoom_by_scales_distance_and_clamps() {
    let mut camera = Camera::default();
    let start = camera.distance;
    // factor > 1 pulls the eye in (distance divides down)...
    camera.zoom_by(2.0);
    assert!((camera.distance - start / 2.0).abs() < 1e-4);
    // ...and factor < 1 pushes it back out.
    camera.zoom_by(0.5);
    assert!((camera.distance - start).abs() < 1e-4);
    // A huge factor clamps at the near limit; a tiny one at the far limit.
    camera.zoom_by(1e6);
    assert_eq!(camera.distance, Camera::MIN_DISTANCE);
    camera.zoom_by(1e-6);
    assert_eq!(camera.distance, Camera::MAX_DISTANCE);
    // Non-positive factors are ignored (no divide-by-zero or sign flip).
    let held = camera.distance;
    camera.zoom_by(0.0);
    camera.zoom_by(-3.0);
    assert_eq!(camera.distance, held);
}

#[test]
fn visible_count_matches_visible_positions() {
    // `visible_count` is a `Vec::with_capacity` hint; it must equal the
    // number `visible_positions` actually enumerates, including the
    // degenerate cases where a non-positive extent collapses an axis to
    // empty.
    for &(t, f, s) in &[(0, 0, 0), (2, 1, 0), (3, 3, 3), (1, 0, 4), (-1, 2, 0)] {
        let view = ViewConfig {
            extent_threes: t,
            extent_fives: f,
            extent_sevens: s,
            ..ViewConfig::default()
        };
        assert_eq!(
            view.visible_count(),
            view.visible_positions().count(),
            "extents ({t}, {f}, {s})"
        );
    }
}
