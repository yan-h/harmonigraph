//! What a sounding note puts on its node: the pitch gradient, the fades
//! each layer runs on, and the seed that keeps them from moving together.

use crate::*;
use glam::Vec3;
use harmonigraph_core::{NoteEvent, NoteEventKind, NoteTracker, Tuning};
use super::harness::*;

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
    // Hold C4, tap-and-release C5: the C5 indicator must decay on
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
    assert_eq!(origin.octaves[MIDDLE_C_SLOT], 1.0, "held octave at full");
    assert!(
        origin.octaves[MIDDLE_C_SLOT + 1] > 0.0 && origin.octaves[MIDDLE_C_SLOT + 1] < 0.75,
        "released octave mid-fade, got {}",
        origin.octaves[MIDDLE_C_SLOT + 1]
    );
}

#[test]
fn one_fade_time_carries_the_body_but_the_marks_snap_off() {
    // The core and the octave glyphs ride the single Fade param: release a
    // two-note chord and half a fade later both are half-way down. The
    // melody/bass rings do NOT ride it — they come off with the key, so a
    // released note wears no mark at all even mid-fade.
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
        mark_melody: true,
        mark_bass: true,
        ..ViewConfig::default()
    };
    let scene = scene_of(&tracker, &Tuning::default(), &view, &frame, 1.0);

    let half = |what: &str, v: f32| {
        assert!((v - 0.5).abs() < 1e-5, "{what} should be half-faded, got {v}");
    };
    // C4 sits on the origin node; its body is half-faded...
    let origin = origin_node(&scene);
    half("the core", origin.activation);
    half("the octave glyph", origin.octaves[MIDDLE_C_SLOT]);
    // ...but no ring survives the release, on any node.
    assert!(
        scene.nodes.iter().all(|n| n.melody_slots == 0 && n.bass_slots == 0),
        "released notes wear no melody/bass mark",
    );
    assert!(
        scene.nodes.iter().all(|n| n.melody_level == 0.0 && n.bass_level == 0.0),
        "and no mark level lingers",
    );
}

#[test]
fn releasing_a_chord_leaves_no_fading_marks() {
    // The reported bug: releasing a held chord smeared a fading melody/bass
    // ring across most pitch classes, because each key-lift was measured
    // against the notes still down and so kept re-crowning a new momentary
    // extreme. Now a released note wears no mark at all, so whatever order a
    // chord's keys come up in, nothing is left fading behind them.
    let chord = [60u8, 62, 64, 65, 67]; // C D E F G
    let mut tracker = NoteTracker::new();
    for &note in &chord {
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
    }
    // Lift the keys one at a time, top-down, each a hair apart.
    for (i, &note) in [67u8, 65, 64, 62, 60].iter().enumerate() {
        tracker.handle_event(NoteEvent {
            time: 0.001 * (i as f64 + 1.0),
            channel: 0,
            note,
            kind: NoteEventKind::Off,
        });
    }
    // Mid-fade, well within one fade time.
    let frame = FrameParams { fade_time: 2.0, ..FrameParams::default() };
    let view = ViewConfig {
        mark_melody: true,
        mark_bass: true,
        ..ViewConfig::default()
    };
    let scene = scene_of(&tracker, &Tuning::default(), &view, &frame, 0.5);
    // The discs are still fading (the notes remain visible)...
    assert!(scene.nodes.iter().any(|n| n.activation > 0.0), "discs still fading");
    // ...but not one melody or bass ring survives the release.
    for n in &scene.nodes {
        assert_eq!(n.melody_slots, 0, "no melody ring on a released chord");
        assert_eq!(n.bass_slots, 0, "no bass ring on a released chord");
    }
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
    // Sampled after ATTACK_TIME: the octave indicator eases in,
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
    assert_eq!(origin.octaves[MIDDLE_C_SLOT], 1.0);
}

#[test]
fn a_note_outside_the_window_lights_the_outermost_indicator() {
    // A narrow window is a way of READING the music, not a filter over it: an
    // octave the wheel has no indicator for folds into the nearest one it
    // does, so the note is still there to see and only its exact octave is
    // given up. Dropping it instead would make a node go dark for notes that
    // are audibly sounding on it.
    let view = ViewConfig { octave_span: 2, ..ViewConfig::default() };
    // ±2 draws five indicators: middle C's octave and two either side, so
    // MIDI 36..95 — every note from C1 to B5 in the DAW's numbering — has one
    // of its own, and only what is past those folds.
    let lit = |note: u8| {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        let scene = scene_of(&tracker, &Tuning::default(), &view, &FrameParams::default(), 0.5);
        let octaves = origin_node(&scene).octaves;
        let slots: Vec<usize> = (0..OCTAVE_SLOTS).filter(|&s| octaves[s] > 0.0).collect();
        assert_eq!(slots.len(), 1, "one octave sounds, got slots {slots:?}");
        slots[0]
    };
    assert_eq!(lit(60), MIDDLE_C_SLOT, "middle C sounds in its own indicator");
    // Both ends of the Range the setting names, which is the whole of what
    // ±2 claims: every C from MIDI 36 to MIDI 84 lights an indicator of its
    // own, and no two of them share one.
    assert_eq!(lit(36), MIDDLE_C_SLOT - 2, "the bottom of the Range has its own");
    assert_eq!(lit(84), MIDDLE_C_SLOT + 2, "the top of the Range has its own");
    assert_eq!(lit(96), MIDDLE_C_SLOT + 2, "an octave past the top folds into it");
    assert_eq!(lit(24), MIDDLE_C_SLOT - 2, "an octave under the bottom folds into it");
    // The widest window reaches those octaves for real, so the fold is the
    // setting talking and not a ceiling in the packing.
    let wide = ViewConfig { octave_span: 5, ..ViewConfig::default() };
    let mut tracker = NoteTracker::new();
    tracker.handle_event(NoteEvent {
        time: 0.0,
        channel: 0,
        note: 96,
        kind: NoteEventKind::On { velocity: 1.0 },
    });
    let scene = scene_of(&tracker, &Tuning::default(), &wide, &FrameParams::default(), 0.5);
    assert_eq!(
        origin_node(&scene).octaves[MIDDLE_C_SLOT + 3],
        1.0,
        "at the widest window C7 has an indicator of its own"
    );
}

#[test]
fn every_node_draws_the_octaves_the_range_names() {
    // A slot is a MIDI octave, so the Range names octave NUMBERS and every
    // node draws all of them — the same five at ±2 whether the node is a C or
    // a tritone off it. A node's pitch class only says where round the turn
    // its octaves land, never which ones there are, so the fold a note takes
    // is one lookup for the whole frame.
    let layout = octave_layout(2, OctaveTaper::Uniform, 0.0);
    assert_eq!(
        layout.slot_range(),
        (MIDDLE_C_SLOT as u32 - 2, MIDDLE_C_SLOT as u32 + 2),
        "±2 draws middle C's octave and two either side"
    );
    // Where they land does move with the pitch class: an F# node's octaves
    // sit half an octave round from a C node's.
    let (c_edge, _) = layout.sector(MIDDLE_C_SLOT as u32, 0.0);
    let (f_edge, _) = layout.sector(MIDDLE_C_SLOT as u32, 600.0);
    let step = std::f32::consts::TAU / (2.0 * layout.octaves as f32);
    assert!((c_edge - f_edge - step).abs() < 1e-4, "an F# node's indicators sit elsewhere");
}

#[test]
fn the_views_taper_reaches_the_wheel() {
    // The span is pinned by the fold test above, which reads it back through
    // the clamp — but nothing there touches the taper or its amount, so
    // hard-coding either at the derive call would leave the suite green while
    // every ring on screen came out evenly divided.
    let view = ViewConfig {
        octave_span: 3,
        octave_taper: OctaveTaper::Geometric,
        octave_taper_amount: 0.6,
        ..ViewConfig::default()
    };
    let scene = scene_of(
        &sounding(),
        &Tuning::default(),
        &view,
        &FrameParams::default(),
        0.5,
    );
    assert_eq!(
        scene.octave_layout,
        octave_layout(3, OctaveTaper::Geometric, 0.6),
        "the frame's wheel is the one the view asked for"
    );
    assert_ne!(
        scene.octave_layout,
        octave_layout(3, OctaveTaper::Uniform, 0.0),
        "and a taper is not the even division"
    );
}
