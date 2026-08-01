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
fn one_pitch_gives_the_disc_and_the_glyph_one_color() {
    // The shader tints an octave glyph by walking `pitch_ramp_lut`; the disc
    // beneath it is colored on the CPU. They share an edge, which is the
    // harshest test of a color match there is, so the two walks have to agree
    // EXACTLY — a tolerance here is a step someone can see. Reproducing the
    // shader's walk by hand and comparing against the disc path (channel 9 is
    // pitch-gradient) is what pins that.
    //
    // Scoped to ONE pitch down both sides, which is the property the shared
    // table can deliver. What a disc and a glyph are each FED can still differ
    // — derive clamps a voice outside the wheel's Range onto the outermost
    // slot — and that is a different pitch, not a disagreement about a color.
    let lut = pitch_ramp_lut();
    let (dark, bright) = (24.0f32, 108.0f32);
    // The sweep is insurance rather than coverage: both sides reduce to the
    // same arithmetic today, so it can only fail once someone changes one
    // walk's interpolation form and not the other's — which is precisely the
    // edit that would put the gamut corner back between two shapes.
    let mut pitch = dark;
    while pitch <= bright {
        let t = ((pitch - dark) / (bright - dark)).clamp(0.0, 1.0);
        let f = t * (PITCH_LUT_N - 1) as f32;
        let i0 = f.floor() as usize;
        let i1 = (i0 + 1).min(PITCH_LUT_N - 1);
        let glyph = lut[i0].lerp(lut[i1], f - f.floor());
        let disc = channel_color(9, pitch, dark, bright);
        assert_eq!(glyph, disc, "pitch {pitch}: glyph {glyph:?} vs disc {disc:?}");
        pitch += 0.01;
    }
}

#[test]
fn the_table_tracks_the_curve_it_samples() {
    // Agreement between shapes is structural (the test above); what
    // PITCH_LUT_N buys is how closely the table follows the designed curve.
    // Pin that separately so a future cut to the constant shows up as the
    // gradient drifting off its design rather than as nothing at all.
    //
    // 4.5/255 against the 3.6 the constant currently measures. The slack is
    // there because the worst case is governed by where a sample lands
    // relative to the gamut corner near MIDI 43, so a tweak to the ramp's own
    // constants moves that corner and swings the number without anything being
    // wrong — but it is drawn tight enough to fail every cut a person would
    // actually make: 48 measures 9.0/255, 32 measures 9.6, 24 measures 6.2,
    // and 16 — where this started — measures 14.9.
    let (dark, bright) = (24.0f32, 108.0f32);
    let mut worst = 0.0f32;
    let mut worst_pitch = 0.0f32;
    let mut pitch = dark;
    while pitch <= bright {
        let t = ((pitch - dark) / (bright - dark)).clamp(0.0, 1.0);
        let table = pitch_lut_color(pitch, dark, bright);
        let curve = crate::color::designed_pitch_ramp(f64::from(t));
        let e = (table - curve).truncate().abs().max_element();
        if e > worst {
            worst = e;
            worst_pitch = pitch;
        }
        pitch += 0.01;
    }
    assert!(
        worst * 255.0 < 4.5,
        "table strays {:.1}/255 from the designed curve at MIDI {worst_pitch:.2}",
        worst * 255.0
    );
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
    let view = ViewConfig { octave_low: 30.0, octave_high: 90.0, ..ViewConfig::default() };
    // Five octaves centered on middle C, so a C node draws five indicators:
    // middle C's octave and two either side. MIDI 36..95 — every note from C1
    // to B5 in the DAW's numbering — has one of its own, and only what is past
    // those folds.
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
    // Both ends of the window, which is the whole of what it claims: every C
    // from MIDI 36 to MIDI 84 lights an indicator of its own, and no two of
    // them share one.
    assert_eq!(lit(36), MIDDLE_C_SLOT - 2, "the bottom of the Range has its own");
    assert_eq!(lit(84), MIDDLE_C_SLOT + 2, "the top of the Range has its own");
    assert_eq!(lit(96), MIDDLE_C_SLOT + 2, "an octave past the top folds into it");
    assert_eq!(lit(24), MIDDLE_C_SLOT - 2, "an octave under the bottom folds into it");
    // The widest window reaches those octaves for real, so the fold is the
    // setting talking and not a ceiling in the packing.
    let wide = ViewConfig {
        octave_low: crate::PITCH_FLOOR,
        octave_high: crate::PITCH_CEIL,
        ..ViewConfig::default()
    };
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
fn each_node_draws_its_own_octaves_that_land_in_the_window() {
    // The window is a pitch RANGE, so which octaves a node has inside it is a
    // question about that node's pitch class. Where the window is a whole
    // number of octaves every class has the same COUNT of them, and only the
    // numbers shift: five octaves centered on middle C gives a C node slots
    // 3..7 and a G node the five that straddle those.
    let whole = octave_layout(30.0, 90.0, 0.0, DEFAULT_TAPER_SHAPE);
    assert_eq!(
        whole.slot_range(0.0),
        (MIDDLE_C_SLOT as u32 - 2, MIDDLE_C_SLOT as u32 + 2),
        "a C node draws middle C's octave and two either side"
    );
    let (low, high) = whole.slot_range(700.0);
    assert_eq!(high - low, 4, "a G node draws five as well");
    assert_ne!((low, high), whole.slot_range(0.0), "just not the same five");
    // Where they land moves with the pitch class too: an F# node's octaves sit
    // half an octave round from a C node's.
    let (c_edge, _) = whole.sector(MIDDLE_C_SLOT as u32, 0.0);
    let (f_edge, _) = whole.sector(MIDDLE_C_SLOT as u32, 600.0);
    let step = std::f32::consts::TAU / (2.0 * whole.cells as f32);
    assert!((c_edge - f_edge - step).abs() < 1e-4, "an F# node's indicators sit elsewhere");

    // Half an octave more, and the COUNT stops agreeing: MIDI 96 is the sixth
    // C in the window, and a node a semitone above C has no sixth of its own
    // to draw there. Which is the whole reason the set is per node — the ring
    // closes either way (see `the_indicators_tile_the_turn`), and forcing both
    // to draw six would leave one of them overlapping itself.
    let part = octave_layout(30.0, 96.0, 0.0, DEFAULT_TAPER_SHAPE);
    let count = |cents: f32| {
        let (low, high) = part.slot_range(cents);
        high - low + 1
    };
    assert_eq!(count(0.0), 6, "a C node reaches MIDI 96");
    assert_eq!(count(100.0), 5, "a C# node's sixth octave is past the window");
}

#[test]
fn the_views_taper_reaches_the_wheel() {
    // The window is pinned by the fold test above, which reads it back through
    // the clamp — but nothing there touches the taper or its amount, so
    // hard-coding either at the derive call would leave the suite green while
    // every ring on screen came out evenly divided.
    let view = ViewConfig {
        octave_low: 18.0,
        octave_high: 102.0,
        octave_taper_amount: 0.6,
        octave_taper_shape: 0.25,
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
        octave_layout(18.0, 102.0, 0.6, 0.25),
        "the frame's wheel is the one the view asked for"
    );
    assert_ne!(
        scene.octave_layout,
        octave_layout(18.0, 102.0, 0.0, DEFAULT_TAPER_SHAPE),
        "and a taper is not the even division"
    );
}

#[test]
fn a_degenerate_color_range_lands_where_the_shader_lands() {
    // The two ends of the gradient are independent params over 0..120 (the
    // 12-semitone ordering is the range bar's, not the param's), so a host
    // reaches ranges the UI cannot draw: inverted, and collapsed to a point.
    // The shader takes both without complaint — it clamps the RATIO
    // (`pitch_lut_color`, lattice.wgsl) — so the CPU has to land where it
    // lands, or a mark is painted one end of the ramp while the very glyph it
    // brackets is painted the other.
    let shader_t = |pitch: f32, dark: f32, bright: f32| {
        ((pitch - dark) / (bright - dark).max(0.01)).clamp(0.0, 1.0)
    };
    for (dark, bright) in [(24.0f32, 108.0f32), (60.0, 60.0), (110.0, 108.0), (108.0, 24.0)] {
        for pitch in [0.0f32, 36.0, 60.0, 72.0, 108.0, 120.0] {
            let lut = pitch_ramp_lut();
            let f = shader_t(pitch, dark, bright) * (PITCH_LUT_N - 1) as f32;
            let i0 = f.floor() as usize;
            let want = lut[i0].lerp(lut[(i0 + 1).min(PITCH_LUT_N - 1)], f - f.floor());
            assert_eq!(
                pitch_lut_color(pitch, dark, bright),
                want,
                "pitch {pitch} over range {dark}..{bright}"
            );
        }
    }
}

#[test]
fn an_inverted_color_range_still_derives_a_scene() {
    // The mark color runs the ramp for EVERY channel, so a fixed-color
    // channel now reaches gradient math that only channels 9-13 used to —
    // and Darkest above Brightest is one drag of a host's parameter list
    // away (raise Darkest before lowering Brightest and it is the state in
    // between). Deriving must not panic there.
    let frame =
        FrameParams { darkest_pitch: 110.0, brightest_pitch: 108.0, ..FrameParams::default() };
    let view = ViewConfig { mark_melody: true, mark_bass: true, ..ViewConfig::default() };
    let scene = scene_of(&held(60), &Tuning::default(), &view, &frame, 0.0);
    let origin = origin_node(&scene);
    assert!(origin.melody_color.is_finite(), "a mark color must be a color");
}

/// The octave indicator lit by a note stands for the pitch that note has ON
/// THIS NODE, across the octave wrap.
///
/// The slot is chosen from the pitch a node will DRAW the indicator at —
/// `slot_pitch(slot, node_cents)` — and a node's pitch class only has to agree
/// with the voice's to within `Tuning::tolerance`, a comparison that WRAPS at
/// the octave. Taking the slot from the voice's own MIDI octave instead is
/// right only while the two sit on the same side of that wrap; when they
/// straddle it the lit sector is a whole octave-slice away from the note that
/// lit it.
///
/// A C offset a shade below zero is the ordinary way in: it puts the origin's
/// pitch class just UNDER 1200 while a played C is at 0, which the wrap still
/// matches. The offset is a host parameter over -600..600, and Learn writes it
/// straight through, so a fraction of a cent flat is a setting rather than a
/// hand-edit.
#[test]
fn a_lit_octave_indicator_stands_for_the_pitch_it_is_drawn_at() {
    // 12-TET, the origin 0.4c flat, matched within the default 0.5c tolerance.
    let tuning = Tuning::from_cents(-0.4, 700.0, 400.0, 1000.0, 0.5);
    let view = ViewConfig::default();
    let scene = scene_of(&held(60), &tuning, &view, &FrameParams::default(), 0.5);
    let origin = origin_node(&scene);

    let node_cents = tuning.pitch_class(LatticePos::ORIGIN).to_cents();
    assert!(node_cents > 1199.0, "the origin must sit just under the wrap, got {node_cents}");

    let lit: Vec<usize> = (0..OCTAVE_SLOTS).filter(|&s| origin.octaves[s] > 0.0).collect();
    assert_eq!(lit.len(), 1, "one octave sounds, got {lit:?}");

    // The indicator a C4 lights must be the one whose own pitch is a C4 on
    // this node -- 59.996, not the 71.996 that the slot above stands for.
    let drawn = scene.octave_layout.slot_pitch(lit[0] as u32, node_cents);
    assert!(
        (drawn - 60.0).abs() < 0.5,
        "a C4 lit the indicator for pitch {drawn}, an octave off the note that lit it",
    );

    // And the ring takes the colour of the sector it brackets, so a slot an
    // octave out is also a ring a seventh of the ramp away from the disc it
    // sits on -- the mismatch the one colour table exists to have ruled out.
    let marked: Vec<usize> =
        (0..OCTAVE_SLOTS).filter(|&s| origin.melody_slots >> s & 1 == 1).collect();
    assert_eq!(marked, lit, "the melody ring marks the octave that sounds");
    let frame = FrameParams::default();
    let want = pitch_lut_color(drawn, frame.darkest_pitch, frame.brightest_pitch);
    assert!(
        (origin.melody_color - want).length() < 1e-5,
        "the ring is coloured for a slot the indicator is not drawn at",
    );
}
