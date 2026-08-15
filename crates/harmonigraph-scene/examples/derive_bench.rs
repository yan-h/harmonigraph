//! Scratch measurement harness for the lattice optimization audit: times the
//! per-frame scene derivation and the per-event tracker path at several
//! node-count x voice-count points, including adversarial ones. Not a test;
//! run with
//! `cargo run --release -p harmonigraph-scene --example derive_bench`.

use std::time::Instant;

use harmonigraph_core::{NoteEvent, NoteEventKind, NoteTracker, Tuning};
use harmonigraph_scene::{derive_scene, Camera, FrameParams, ViewConfig};

/// A tracker holding `held` simultaneous notes plus a released tail from a
/// fast monophonic line: `rate` notes/second for the last `tail_secs` seconds,
/// each released just after its successor arrived, none pruned yet (fade 1s,
/// attack 1s — the released tail's worst case under the Fade param's cap).
fn tracker_with(held: usize, rate: usize, tail_secs: f64, now: f64) -> NoteTracker {
    let mut tracker = NoteTracker::new();
    // The tail first: a run of notes each a step apart, walking the keyboard,
    // every one of them briefly the highest AND lowest held note, so each
    // carries a melody and a bass stamp — the marking worst case.
    let count = (rate as f64 * tail_secs) as usize;
    for i in 0..count {
        let t = now - tail_secs + i as f64 / rate as f64;
        let note = 24 + (i % 84) as u8;
        let channel = (i / 84 % 16) as u8;
        tracker.handle_event(NoteEvent::on(t, channel, note, 0.8));
        tracker.handle_event(NoteEvent::off(t + 0.8 / rate as f64, channel, note));
    }
    // The held chord on top.
    for i in 0..held {
        let note = 36 + (i % 60) as u8;
        let channel = (i / 60) as u8;
        tracker.handle_event(NoteEvent::on(now - 0.5, channel, note, 0.9));
    }
    tracker
}

/// Fill the history to its 384-visit cap with microtonally distinct pitches,
/// so the trail pass runs at full width.
fn fill_history(tracker: &mut NoteTracker, env: &harmonigraph_core::Envelope) {
    for i in 0..400u32 {
        let t = -2000.0 + f64::from(i);
        let note = 24 + (i % 84) as u8;
        tracker.handle_event(NoteEvent::on(t, 0, note, 0.8));
        tracker.handle_event(NoteEvent {
            time: t,
            channel: 0,
            note,
            kind: NoteEventKind::Tuning { semitones: (i / 84) as f32 * 0.03 },
        });
        tracker.handle_event(NoteEvent::off(t + 0.5, 0, note));
    }
    tracker.prune(-100.0, env);
}

fn time_derive(label: &str, tracker: &NoteTracker, view: &ViewConfig, frame: &FrameParams, now: f64) {
    let tuning = Tuning::default();
    let camera = Camera::default();
    // Warm up (LUT memo, allocator).
    for _ in 0..3 {
        let _ = derive_scene(tracker, &tuning, view, frame, camera, None, now);
    }
    let reps = 20;
    let start = Instant::now();
    let mut nodes = 0;
    for _ in 0..reps {
        let scene = derive_scene(tracker, &tuning, view, frame, camera, None, now);
        nodes = scene.nodes.len();
    }
    let ms = start.elapsed().as_secs_f64() * 1000.0 / f64::from(reps);
    let voices = tracker.voices().count();
    println!("{label:<46} {nodes:>6} nodes  {voices:>4} voices  {ms:>8.3} ms/frame");
}

fn main() {
    let now = 10_000.0f64;
    let frame = FrameParams { fade_time: 1.0, ..FrameParams::default() };
    let env = harmonigraph_core::Envelope { attack_time: 1.0, fade_time: 1.0, shape: 0.0 };

    // Views: the default window, a 16:9 zoomed-out flat pane (~875 nodes per
    // the MAX_DRAWN_NODES doc), and the pathological wide-cabinet one.
    let small = ViewConfig { extent_threes: 5, extent_fives: 4, ..ViewConfig::default() };
    let default_view = ViewConfig::default();
    let huge = ViewConfig {
        extent_threes: 40,
        extent_fives: 20,
        extent_sevens: 2,
        ..ViewConfig::default()
    };
    println!(
        "views: small={} default={} huge={}",
        small.visible_count(),
        default_view.visible_count(),
        huge.visible_count()
    );

    println!("\n-- derive_scene: idle / chord / burst / worst tail (trail OFF) --");
    let mut no_trail = default_view.clone();
    no_trail.trail_labels = false;
    let mut small_nt = small.clone();
    small_nt.trail_labels = false;
    let mut huge_nt = huge.clone();
    huge_nt.trail_labels = false;

    let idle = NoteTracker::new();
    time_derive("idle, default window", &idle, &no_trail, &frame, now);

    let chord = tracker_with(10, 0, 0.0, now);
    time_derive("10-note chord, small window", &chord, &small_nt, &frame, now);
    time_derive("10-note chord, default window", &chord, &no_trail, &frame, now);
    time_derive("10-note chord, huge window", &chord, &huge_nt, &frame, now);

    let burst = tracker_with(60, 0, 0.0, now);
    time_derive("60 simultaneous notes, default window", &burst, &no_trail, &frame, now);
    time_derive("60 simultaneous notes, huge window", &burst, &huge_nt, &frame, now);

    // 30 notes/sec for 2 seconds of tail (fade+attack both 1s) + 10 held.
    let busy = tracker_with(10, 30, 2.0, now);
    time_derive("10 held + 60 released (30/s line)", &busy, &no_trail, &frame, now);
    let frantic = tracker_with(10, 100, 2.0, now);
    time_derive("10 held + 200 released (100/s line)", &frantic, &no_trail, &frame, now);
    time_derive("10 held + 200 released, huge window", &frantic, &huge_nt, &frame, now);

    println!("\n-- same, with the trail at its 384-visit cap (trail ON) --");
    let mut chord_hist = tracker_with(10, 0, 0.0, now);
    fill_history(&mut chord_hist, &env);
    time_derive("10-note chord, default window", &chord_hist, &default_view, &frame, now);
    time_derive("10-note chord, huge window", &chord_hist, &huge, &frame, now);
    let mut frantic_hist = tracker_with(10, 100, 2.0, now);
    fill_history(&mut frantic_hist, &env);
    time_derive("10 held + 200 released, default window", &frantic_hist, &default_view, &frame, now);
    time_derive("10 held + 200 released, huge window", &frantic_hist, &huge, &frame, now);

    println!("\n-- tracker event path --");
    // A realistic burst: a 10-note chord arriving in one drained batch.
    let mut tracker = tracker_with(0, 30, 2.0, now);
    let start = Instant::now();
    let reps = 1000;
    for r in 0..reps {
        let t = now + r as f64 * 0.001;
        for note in [48u8, 52, 55, 59, 62, 65, 69, 72, 76, 79] {
            tracker.handle_event(NoteEvent::on(t, 0, note, 0.9));
        }
        for note in [48u8, 52, 55, 59, 62, 65, 69, 72, 76, 79] {
            tracker.handle_event(NoteEvent::off(t + 0.0005, 0, note));
        }
    }
    let us = start.elapsed().as_secs_f64() * 1e6 / f64::from(reps);
    println!("10-note chord on+off (20 events):      {us:>8.1} us/batch");

    // The adversarial flood: the ring's full 4096 events, all note-ons on
    // distinct keys, drained into one frame while ~2048 keys end up held.
    let mut tracker = NoteTracker::new();
    let start = Instant::now();
    for i in 0..4096u32 {
        let channel = (i / 128 % 16) as u8;
        let note = (i % 128) as u8;
        tracker.handle_event(NoteEvent::on(now + f64::from(i) * 1e-5, channel, note, 0.5));
    }
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    println!("4096-event note-on flood (2048 keys):  {ms:>8.2} ms/drain");
    let start = Instant::now();
    tracker.prune(now + 100.0, &env);
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    println!("prune after the flood:                 {ms:>8.2} ms");
}
