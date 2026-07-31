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
    half("the octave glyph", origin.octaves[4]);
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
    assert_eq!(origin.octaves[4], 1.0);
}
