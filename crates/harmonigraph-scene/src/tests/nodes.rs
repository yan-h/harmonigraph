//! What a sounding note puts on its node: the pitch gradient, the fades
//! each layer runs on, and the seed that keeps them from moving together.

use crate::*;
use glam::Vec3;
use harmonigraph_core::{NoteEvent, NoteEventKind, NoteTracker, Tuning};
use super::harness::*;

#[test]
fn pitch_colored_channels_vary_with_pitch() {
    let p = PitchPalette::Ramp;
    let low = channel_color(9, 24.0, 24.0, 108.0, p);
    let high = channel_color(9, 108.0, 24.0, 108.0, p);
    assert_ne!(low, high);
    // On Ramp — the palette where brightness is what carries the pitch — the
    // top of the range is, well, brighter. Scoped to Ramp on purpose: the
    // fixed-lightness palettes deliberately break exactly this, which
    // `every_flat_palette_is_in_gamut_and_isoluminant` pins from the other
    // side.
    assert!(high.truncate().length() > low.truncate().length());
}

/// sRGB relative luminance — what "brightness" means once a color is on
/// screen, and the thing the flat palettes hold still. Not `L*`: holding `L*`
/// is how they are DEFINED, so measuring `L*` back would only restate the
/// definition. This goes the whole way to the pixel, through the gamut clamp
/// included, which is where a chroma set too high would show up.
fn luminance(c: glam::Vec4) -> f32 {
    let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * lin(c.x) + 0.7152 * lin(c.y) + 0.0722 * lin(c.z)
}

/// Each palette's whole reason for existing, as a number: how far its
/// luminance moves from the bottom of the gradient to the top.
///
/// This is the test that catches a chroma nudged past what the sRGB gamut
/// holds. A clipped channel does not announce itself — the color simply stops
/// being the LCh that was asked for — but clipping cannot happen without
/// moving the luminance, so a flat palette that clips anywhere is a flat
/// palette whose luminance is no longer flat, and the ratio below rises off
/// 1.0 the moment it does.
#[test]
fn every_flat_palette_is_in_gamut_and_isoluminant() {
    // The three fixed-`L*` palettes are exactly flat (1.0); Lift and Ramp are
    // the tilted ones, and the gap between their two spans is the choice
    // being offered. Ember leans hardest of all.
    //
    // "Flat" is 1.0 up to 1.01, not bit-exact: the LCh->sRGB conversion
    // rounds, and it rounds differently at different chroma, so a fixed-`L*`
    // arc comes out within 1.0014 (Ink, the widest chroma swing) rather than
    // dead level. That is four hundred times under a step anyone can see, and
    // a real design tilt would be worth percent, not tenths of one.
    for (palette, lo_bound, hi_bound) in [
        (PitchPalette::Even, 1.0, 1.01),
        (PitchPalette::Neon, 1.0, 1.01),
        (PitchPalette::Ink, 1.0, 1.01),
        (PitchPalette::Lift, 2.0, 3.5),
        (PitchPalette::Ramp, 5.0, 6.0),
        (PitchPalette::Ember, 6.5, 8.0),
    ] {
        let lut = pitch_ramp_lut(palette);
        let (mut lo, mut hi) = (f32::MAX, 0.0f32);
        for entry in lut {
            // In gamut on both sides for every palette, flat or not: the LUT
            // is what the shader indexes, so an entry outside 0..1 would be a
            // color no pass can draw.
            for ch in [entry.x, entry.y, entry.z] {
                assert!((0.0..=1.0).contains(&ch), "{palette:?}: {entry:?} is outside sRGB");
            }
            let y = luminance(entry);
            lo = lo.min(y);
            hi = hi.max(y);
        }
        let ratio = hi / lo;
        assert!(
            ratio >= lo_bound - 1e-3 && ratio <= hi_bound + 1e-3,
            "{palette:?} spans {ratio:.4}x in luminance, wanted {lo_bound}..{hi_bound}",
        );
    }
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
    //
    // Every palette, because agreement is structural — one table read by both
    // walks — and a palette that broke it would be one that reached a color
    // some other way than through the table.
    let (dark, bright) = (24.0f32, 108.0f32);
    for palette in PitchPalette::ALL {
        let lut = pitch_ramp_lut(palette);
        // The sweep is insurance rather than coverage: both sides reduce to
        // the same arithmetic today, so it can only fail once someone changes
        // one walk's interpolation form and not the other's — which is
        // precisely the edit that would put the gamut corner back between two
        // shapes.
        let mut pitch = dark;
        while pitch <= bright {
            let t = ((pitch - dark) / (bright - dark)).clamp(0.0, 1.0);
            let f = t * (PITCH_LUT_N - 1) as f32;
            let i0 = f.floor() as usize;
            let i1 = (i0 + 1).min(PITCH_LUT_N - 1);
            let glyph = lut[i0].lerp(lut[i1], f - f.floor());
            let disc = channel_color(9, pitch, dark, bright, palette);
            assert_eq!(glyph, disc, "{palette:?} pitch {pitch}: glyph {glyph:?} vs disc {disc:?}");
            pitch += 0.01;
        }
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
        let table = pitch_lut_color(pitch, dark, bright, PitchPalette::Ramp);
        let curve = crate::color::designed_pitch_ramp(f64::from(t), PitchPalette::Ramp);
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
fn a_note_outside_the_ring_lights_the_outermost_indicator() {
    // A narrow span is a way of READING the music, not a filter over it: an
    // octave the wheel has no indicator for folds into the nearest one it
    // does, so the note is still there to see and only its exact octave is
    // given up. Dropping it instead would make a node go dark for notes that
    // are audibly sounding on it.
    // `octave_extras: 0` explicitly rather than `ViewConfig::default()`: the
    // fresh-view look is Yan's and is free to ship a fringe, but this test is
    // about the fold at the ring's own edge, which a fringe would move.
    let view =
        ViewConfig { octave_count: 5, octave_center: 60.0, octave_extras: 0, ..ViewConfig::default() };
    // Five octaves with middle C at the top, so a C node draws five
    // indicators: middle C's octave and two either side. MIDI 36..95 — every
    // note from C1 to B5 in the DAW's numbering — has one of its own, and only
    // what is past those folds.
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
    // Both ends of the ring, which is the whole of what it claims: every C
    // from MIDI 36 to MIDI 84 lights an indicator of its own, and no two of
    // them share one.
    assert_eq!(lit(36), MIDDLE_C_SLOT - 2, "the bottom of the ring has its own");
    assert_eq!(lit(84), MIDDLE_C_SLOT + 2, "the top of the ring has its own");
    assert_eq!(lit(96), MIDDLE_C_SLOT + 2, "an octave past the top folds into it");
    assert_eq!(lit(24), MIDDLE_C_SLOT - 2, "an octave under the bottom folds into it");
    // The widest span reaches those octaves for real, so the fold is the
    // setting talking and not a ceiling in the packing.
    let wide = ViewConfig {
        octave_count: crate::MAX_SPAN,
        octave_center: 60.0,
        octave_extras: 0,
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
        "at the widest span C7 has an indicator of its own"
    );
}

#[test]
fn a_ring_reaching_under_the_packing_folds_onto_a_slot_it_has() {
    // A ring near the bottom of the pitch limits draws octaves no MIDI note
    // can reach (see `Ring::base`), so a note folding outward has to land on
    // the outermost indicator that HAS a slot, not on the outermost one
    // drawn. At any ordinary center every octave the ring names is a real
    // one and the second clamp in `derive_scene` never moves anything — this
    // is the case that makes it load-bearing, and without it the fold indexes
    // the packing at -1.
    //
    // The origin a shade under the wrap is what puts a played C an octave
    // below the node's own numbering: `matches` wraps, so a node at 1195¢ is
    // lit by a played 0¢, and the lowest MIDI C on it comes out as slot -1.
    let tuning = Tuning::from_cents(-5.0, 700.0, 400.0, 1000.0, 5.0);
    // `octave_extras: 0` explicitly rather than `ViewConfig::default()`: the
    // fresh-view look is Yan's and is free to ship a fringe, but this test is
    // about the packing's own edge, which a fringe would move.
    let view =
        ViewConfig { octave_count: 5, octave_center: 12.0, octave_extras: 0, ..ViewConfig::default() };
    let scene = scene_of(&held(0), &tuning, &view, &FrameParams::default(), 0.5);
    let node_cents = tuning.pitch_class(LatticePos::ORIGIN).to_cents();
    assert!(node_cents > 1190.0, "the origin must sit just under the wrap, got {node_cents}");
    assert_eq!(
        scene.octave_layout.slots(node_cents).0,
        -2,
        "the ring has to reach under the packing, or this tests nothing"
    );
    let origin = origin_node(&scene);
    assert_eq!(origin.activation, 1.0, "the played C lights the node it wraps onto");
    let lit: Vec<usize> = (0..OCTAVE_SLOTS).filter(|&s| origin.octaves[s] > 0.0).collect();
    assert_eq!(lit, [0], "the note folds onto the lowest slot the packing has");
}

#[test]
fn each_node_draws_its_own_octaves_nearest_the_center() {
    // Which octaves a node draws is a question about that node's pitch class,
    // and the COUNT is not: every class draws the span, so the numbers shift
    // and nothing else does. Five octaves centered on middle C gives a C node
    // slots 3..7 and a G node the five that straddle those.
    let wheel = octave_layout(5, 60.0, 0, DEFAULT_EXTRA_SIZE, DEFAULT_EXTRA_BLEND);
    assert_eq!(
        wheel.slots(0.0),
        (MIDDLE_C_SLOT as i32 - 2, MIDDLE_C_SLOT as i32 + 2),
        "a C node draws middle C's octave and two either side"
    );
    let g = wheel.slots(700.0);
    assert_eq!(g.1 - g.0, 4, "a G node draws five as well");
    assert_ne!(g, wheel.slots(0.0), "just not the same five");

    // An EVEN span has no symmetric answer, so which side the extra octave
    // falls is a decision rather than arithmetic: it goes on the side of the
    // node's nearest octave the center itself sits, and a center landing
    // exactly on one of the node's octaves breaks the tie downward. Pinned
    // absolutely, because everything else here is measured FROM the ring's
    // base and would read the same with the whole ring an octave out.
    let even = |center: f32| {
        octave_layout(4, center, 0, DEFAULT_EXTRA_SIZE, DEFAULT_EXTRA_BLEND).slots(0.0)
    };
    assert_eq!(even(59.0), (3, 6), "a center under the node's octave reaches down");
    assert_eq!(even(61.0), (4, 7), "and one over it reaches up");
    assert_eq!(even(60.0), (3, 6), "a center on the octave itself ties downward");

    // Where they land moves with the pitch class too, and by exactly the pitch
    // distance: a G is five semitones under the C above it, so its ring turns
    // left by five semitones' worth of the turn — half a slice at the tritone
    // and never further, which is what keeps the picture from lurching as a
    // chord walks up the lattice.
    let turn = |cents: f32| wheel.ring(cents).seam - wheel.ring(0.0).seam;
    let semitone = std::f32::consts::TAU / 60.0;
    assert!((turn(700.0) - 5.0 * semitone).abs() < 1e-4, "a G node turned the wrong way");
    assert!((turn(200.0) + 2.0 * semitone).abs() < 1e-4, "a D node turned the wrong way");
    assert!((turn(600.0).abs() - 6.0 * semitone).abs() < 1e-4, "an F# node is half a slice round");
}

#[test]
fn the_views_fringe_reaches_the_wheel() {
    // The count and center are pinned by the fold test above, which reads them
    // back through the clamp — but nothing there touches the extras or their
    // size, so hard-coding any of the three at the derive call would leave the
    // suite green while every ring on screen came out evenly divided.
    let view = ViewConfig {
        octave_count: 5,
        octave_center: 60.0,
        octave_extras: 2,
        octave_extra_size: 0.4,
        octave_extra_blend: 0.25,
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
        octave_layout(5, 60.0, 2, 0.4, 0.25),
        "the frame's wheel is the one the view asked for"
    );
    assert_ne!(
        scene.octave_layout,
        octave_layout(5, 60.0, 0, 0.4, 0.25),
        "and a fringe is not the even division"
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
            let lut = pitch_ramp_lut(PitchPalette::Ramp);
            let f = shader_t(pitch, dark, bright) * (PITCH_LUT_N - 1) as f32;
            let i0 = f.floor() as usize;
            let want = lut[i0].lerp(lut[(i0 + 1).min(PITCH_LUT_N - 1)], f - f.floor());
            assert_eq!(
                pitch_lut_color(pitch, dark, bright, PitchPalette::Ramp),
                want,
                "pitch {pitch} over range {dark}..{bright}"
            );
        }
    }
}

#[test]
fn an_inverted_color_range_still_derives_a_scene() {
    // The mark color runs the ramp for EVERY channel, so a fixed-color
    // channel reaches the same gradient math a pitch-gradient one does —
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
    let drawn = scene.octave_layout.slot_pitch(lit[0] as i32, node_cents);
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
    let want =
        pitch_lut_color(drawn, frame.darkest_pitch, frame.brightest_pitch, PitchPalette::Ramp);
    assert!(
        (origin.melody_color - want).length() < 1e-5,
        "the ring is coloured for a slot the indicator is not drawn at",
    );
}
