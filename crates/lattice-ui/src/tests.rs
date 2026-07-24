//! Unit tests for the UI shell.

use super::*;

#[test]
fn persist_round_trips_camera_and_view() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.camera.yaw = 1.23;
    state.camera.distance = 42.0;
    state.view.extent_sevens = 3;
    // Non-default values throughout, so the fields prove they
    // round-trip rather than matching the defaults by luck.
    state.view.outer_style = lattice_scene::OuterStyle::Off;
    // Radius 0 is the off state; this proves it (and solidity) persist.
    state.view.core_radius = 0.0;
    state.view.core_solidity = 0.4;
    state.view.outer_inner = 0.1;
    state.view.outer_outer = 0.7;
    state.view.outer_backdrop = 0.62;
    state.view.outer_solidity = 0.3;
    state.view.idle_marker = lattice_scene::IdleMarker::Dot;
    state.view.idle_radius = 0.31;
    // Melody, not Both: Both is the default, and this test's whole
    // point is that the fields prove they round-trip rather than
    // matching the defaults by luck.
    state.view.highlight_extremes = lattice_scene::HighlightExtremes::Melody;
    state.view.grid_color = [0.9, 0.1, 0.4, 0.25];
    state.view.grid_thickness = 2.5;
    state.view.grid_inset = 0.0;
    state.view.grid_dashed = true;
    state.view.meantone = true;
    state.camera_presets.push(CameraPreset {
        name: "reading".into(),
        yaw: 0.7,
        pitch: 0.2,
    });
    let saved = state.save_persist();

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&saved);
    assert_eq!(restored.camera.yaw, 1.23);
    assert_eq!(restored.camera.distance, 42.0);
    assert_eq!(restored.view.extent_sevens, 3);
    assert_eq!(restored.view.outer_style, lattice_scene::OuterStyle::Off);
    assert_eq!(restored.view.core_radius, 0.0, "off (radius 0) round-trips");
    assert_eq!(restored.view.core_solidity, 0.4);
    assert_eq!(restored.view.outer_inner, 0.1);
    assert_eq!(restored.view.outer_outer, 0.7);
    assert_eq!(restored.view.outer_backdrop, 0.62);
    assert_eq!(restored.view.outer_solidity, 0.3);
    assert_eq!(restored.view.idle_marker, lattice_scene::IdleMarker::Dot);
    assert_eq!(restored.view.idle_radius, 0.31);
    assert_eq!(
        restored.view.highlight_extremes,
        lattice_scene::HighlightExtremes::Melody
    );
    assert_eq!(restored.view.grid_color, [0.9, 0.1, 0.4, 0.25]);
    assert_eq!(restored.view.grid_thickness, 2.5);
    assert_eq!(restored.view.grid_inset, 0.0, "0 (lines to the center) round-trips");
    assert!(restored.view.grid_dashed);
    assert!(restored.view.meantone);
    assert_eq!(restored.camera_presets.len(), 1);
    assert_eq!(restored.camera_presets[0].name, "reading");
    assert_eq!(restored.camera_presets[0].yaw, 0.7);
}

#[test]
fn removed_node_styles_in_old_persist_blobs_load_as_steady() {
    // Breathe/Sparks and the later-trimmed Wire/Corona/… set no longer
    // exist; serde aliases must absorb them so an old blob still restores
    // (a failed parse would silently drop the WHOLE persist — layout,
    // camera, everything). "Wire" is one of the removed names.
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.camera.yaw = 1.23;
    state.view.node_style = lattice_scene::NodeStyle::Vortex;
    let saved = state.save_persist().replace("node_style:Vortex", "node_style:Wire");
    assert_ne!(saved, state.save_persist(), "replacement must have hit");

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&saved);
    assert_eq!(restored.view.node_style, lattice_scene::NodeStyle::Steady);
    assert_eq!(restored.camera.yaw, 1.23, "rest of the blob still restores");
}

#[test]
fn removed_octave_styles_in_old_persist_blobs_load_as_slices() {
    // Dots and Rings joined Petals/Flares/Bumps as removed styles: one
    // glyph shape is left, so none of these exist as variants and serde
    // aliases must absorb each. Without them an old blob doesn't just lose
    // its style, it fails to parse and drops the WHOLE persist (layout,
    // camera and all). Inject the dead tokens as strings, since the enum
    // can no longer name them.
    for removed in ["Dots", "Rings", "Petals", "Flares", "Bumps"] {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        state.view.outer_style = lattice_scene::OuterStyle::Slices;
        let saved = state
            .save_persist()
            .replace("outer_style:Slices", &format!("outer_style:{removed}"));
        assert_ne!(saved, state.save_persist(), "replacement must have hit for {removed}");

        let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
        restored.load_persist(&saved);
        assert_eq!(
            restored.view.outer_style,
            lattice_scene::OuterStyle::Slices,
            "{removed} folds to the one surviving style"
        );
    }
}

#[test]
fn pre_rename_octave_style_and_slice_band_fields_still_load() {
    // The outer layer's fields were renamed (octave_style ->
    // outer_style, slice_inner/outer -> outer_inner/outer); aliases
    // must keep blobs with the old names loading.
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.view.outer_style = lattice_scene::OuterStyle::Slices;
    state.view.outer_inner = 0.25;
    state.view.outer_outer = 0.85;
    let saved = state
        .save_persist()
        .replace("outer_style:", "octave_style:")
        .replace("outer_inner:", "slice_inner:")
        .replace("outer_outer:", "slice_outer:");
    assert_ne!(saved, state.save_persist(), "replacements must have hit");

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&saved);
    assert_eq!(restored.view.outer_style, lattice_scene::OuterStyle::Slices);
    assert_eq!(restored.view.outer_inner, 0.25);
    assert_eq!(restored.view.outer_outer, 0.85);
}

#[test]
fn pre_reorg_layout_keeps_its_settings_and_refreshes_the_dock() {
    // The settings tabs were renamed and split (View -> Frame, Appearance ->
    // Nodes + Scene, Spectrum -> Analyzer, Render -> Video, plus a new Panel)
    // and the persist blob gained a version. An old blob names the old tabs
    // and has no version. Two things must hold: the `Tab` aliases keep it
    // PARSING (a failed parse silently drops camera/view/spectrum with it),
    // and the absent version refreshes the stale dock so the split-out Scene
    // and Panel tabs aren't stranded off-layout.
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.camera.yaw = 1.23;
    state.view.extent_sevens = 3;
    // Rewrite a current blob into a pre-reorg one: drop the version field
    // (serde reads it back as 0) and rename the tabs to their old spellings,
    // which only the aliases can still resolve. The capitalized tab tokens
    // don't occur elsewhere in the blob, so these replacements are surgical.
    let saved = state
        .save_persist()
        .replacen("version:1,", "", 1)
        .replace("Frame", "View")
        .replace("Nodes", "Appearance")
        .replace("Analyzer", "Spectrum")
        .replace("Video", "Render");
    assert_ne!(saved, state.save_persist(), "the rewrite must have hit");

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&saved);
    // Parsed despite the old tab names, so the dialed-in settings survive.
    assert_eq!(restored.camera.yaw, 1.23, "settings survive the old tab names");
    assert_eq!(restored.view.extent_sevens, 3);
    // The stale arrangement is refreshed to the current default, which has
    // every tab (including the ones an old blob couldn't name).
    assert_eq!(
        ron::to_string(&restored.dock).unwrap(),
        ron::to_string(&default_dock()).unwrap(),
        "a pre-versioning layout resets to the current default dock"
    );
}

#[test]
fn pre_radius_off_core_modes_fold_onto_radius_and_solidity() {
    // Pre-radius-off blobs wrote a `core_style` token the current layout
    // no longer serializes; loading one must fold it into radius (0 =
    // off) + solidity so the look is preserved. Inject the dead token
    // ahead of `core_solidity` (the enum still deserializes it).
    for (token, off, solidity) in
        [("Orb", false, 1.0), ("Glow", false, 0.0), ("None", false, 0.0), ("Empty", true, 1.0)]
    {
        let state = SharedState::new(TextureFormat::Bgra8Unorm);
        let saved = state
            .save_persist()
            .replace("core_solidity:", &format!("core_style:{token},core_solidity:"));
        assert_ne!(saved, state.save_persist(), "injection must have hit for {token}");

        let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
        restored.load_persist(&saved);
        if off {
            assert_eq!(restored.view.core_radius, 0.0, "{token} folds to off");
        } else {
            assert!(restored.view.core_radius > 0.0, "{token} stays on");
            assert_eq!(restored.view.core_solidity, solidity, "{token}");
        }
    }
}

#[test]
fn node_body_experiment_blobs_fold_into_core_and_outer() {
    // Blobs saved by the one-build NodeBody experiment carry a
    // node_body field the current layout no longer writes; loading one
    // must both parse and fold the body into the core/outer split
    // (Beads = the core glow, solidity 0, plus the octave layer with its
    // backdrop). They wrote the legacy core_style:Orb.
    let state = SharedState::new(TextureFormat::Bgra8Unorm);
    let saved = state
        .save_persist()
        .replace("core_solidity:", "core_style:Orb,node_body:Beads,core_solidity:");
    assert_ne!(saved, state.save_persist(), "injection must have hit");

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&saved);
    assert_eq!(restored.view.core_solidity, 0.0, "octave-only body is the glow end");
    assert!(restored.view.core_radius > 0.0, "still on");
    assert_eq!(restored.view.outer_style, lattice_scene::OuterStyle::Slices);
    assert_eq!(
        restored.view.outer_backdrop, 1.0,
        "Beads' cohesion device rides the backdrop, at full strength"
    );
    assert_eq!(
        restored.view.node_body,
        lattice_scene::LegacyNodeBody::Disc,
        "shim consumed on load"
    );
}

#[test]
fn legacy_bool_backdrop_blobs_load_as_an_opacity() {
    // The backdrop was a bool before it became an opacity. A stale bool
    // must not just fail to parse: load_persist drops the WHOLE blob on
    // any error, so the user would silently lose their layout, camera
    // and every other view setting along with it.
    for (token, want) in [("true", 1.0f32), ("false", 0.0)] {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        // A value the legacy bool must overrule either way, so both
        // tokens prove the shim ran rather than matching a default.
        state.view.outer_backdrop = 0.5;
        let saved = state
            .save_persist()
            .replace("core_solidity:", &format!("outer_backdrop:{token},core_solidity:"));
        assert_ne!(saved, state.save_persist(), "injection must have hit");

        let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
        restored.load_persist(&saved);
        assert_eq!(restored.view.outer_backdrop, want, "bool {token}");
        assert_eq!(
            restored.view.legacy_outer_backdrop, None,
            "shim consumed on load"
        );
        // The rest of the blob survived rather than being dropped.
        assert_eq!(restored.view.extent_threes, state.view.extent_threes);
    }
}

#[test]
fn corrupt_persist_is_ignored() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let default_distance = state.camera.distance;
    state.load_persist("not json at all");
    assert_eq!(state.camera.distance, default_distance);
}

#[derive(Default)]
struct RecordingBackend {
    sets: std::cell::RefCell<Vec<(params::ParamKey, f32)>>,
}

impl ParamBackend for RecordingBackend {
    fn get(&self, _key: params::ParamKey) -> f32 {
        0.0
    }
    fn set(&self, key: params::ParamKey, value: f32) {
        self.sets.borrow_mut().push((key, value));
    }
}

/// Drive the real root_ui (dock, hover, everything) with a synthetic wheel
/// event over the lattice pane and return the camera distance after it.
/// `modifiers` picks whether egui routes the wheel to a scroll delta (plain)
/// or a zoom factor (COMMAND, egui's default zoom modifier).
fn distance_after_wheel_over_lattice(modifiers: egui::Modifiers) -> (f32, f32) {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let backend = RecordingBackend::default();
    let start = state.camera.distance;

    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 800.0));
    // A point solidly inside the top-left leaf, which holds the Lattice tab
    // alone (see default_dock): past the tab bar, left of the split.
    let over_lattice = egui::pos2(150.0, 150.0);

    let run_frame = |state: &mut SharedState, ctx: &egui::Context, t: f64, wheel: bool| {
        let mut events = vec![egui::Event::PointerMoved(over_lattice)];
        if wheel {
            events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                // Positive y = scroll up = zoom in (both the scroll and the
                // zoom-factor paths map an upward wheel to a smaller distance).
                delta: egui::vec2(0.0, 1.0),
                phase: egui::TouchPhase::Move,
                modifiers,
            });
        }
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(t),
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, |ui| root_ui(ui, state, &backend, t));
    };

    // Warm-up passes so the pointer registers and egui's top-widget-at-
    // pointer resolution (which reads the previous pass) sees the lattice
    // under the pointer before the wheel pass.
    run_frame(&mut state, &ctx, 0.0, false);
    run_frame(&mut state, &ctx, 1.0 / 60.0, false);
    run_frame(&mut state, &ctx, 2.0 / 60.0, true);

    (start, state.camera.distance)
}

/// Repro for "mouse-wheel scroll to zoom no longer works": a plain wheel over
/// the lattice (egui delivers it as a scroll delta) must zoom in.
#[test]
fn scroll_over_lattice_zooms_the_camera() {
    let (start, after) = distance_after_wheel_over_lattice(egui::Modifiers::NONE);
    assert!(after < start, "plain scroll should zoom in ({start} -> {after})");
}

/// A wheel egui classifies as a zoom gesture (modifier+scroll / trackpad
/// pinch) arrives as `zoom_delta`, not a scroll delta. The lattice must zoom
/// on that too — the old handler only read the scroll delta and did nothing.
#[test]
fn zoom_gesture_over_lattice_zooms_the_camera() {
    let (start, after) = distance_after_wheel_over_lattice(egui::Modifiers::COMMAND);
    assert!(after < start, "zoom-gesture wheel should zoom in ({start} -> {after})");
}

#[test]
fn learn_step_writes_params_only_when_the_chord_changes() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let backend = RecordingBackend::default();
    state.learn_active = true;
    // Hold C and G (a 12-TET fifth: within learn range of just).
    for note in [60u8, 67] {
        state.tracker.handle_event(lattice_core::NoteEvent {
            time: 0.0,
            channel: 0,
            note,
            kind: lattice_core::NoteEventKind::On { velocity: 1.0 },
        });
    }

    learn_step(&mut state, &backend);
    let first = backend.sets.borrow().clone();
    assert!(
        first.iter().any(|(k, v)| *k == params::ParamKey::Three && *v == 700.0),
        "the fifth should be learned from C+G, got {first:?}"
    );

    // Same chord again: change detection must suppress further writes.
    learn_step(&mut state, &backend);
    assert_eq!(backend.sets.borrow().len(), first.len());

    // Disarming clears the memory so re-arming re-learns.
    state.learn_active = false;
    learn_step(&mut state, &backend);
    state.learn_active = true;
    learn_step(&mut state, &backend);
    assert_eq!(backend.sets.borrow().len(), first.len() * 2);
}

/// Hold `notes` as channel-0 voices, each optionally bent by a per-note
/// tuning offset (cents). Used to synthesize just vs 12-TET chords.
fn hold_chord(state: &mut SharedState, notes: &[(u8, f32)]) {
    for &(note, cents) in notes {
        state.tracker.handle_event(lattice_core::NoteEvent {
            time: 0.0,
            channel: 0,
            note,
            kind: lattice_core::NoteEventKind::On { velocity: 1.0 },
        });
        if cents != 0.0 {
            state.tracker.handle_event(lattice_core::NoteEvent {
                time: 0.0,
                channel: 0,
                note,
                kind: lattice_core::NoteEventKind::Tuning { semitones: cents / 100.0 },
            });
        }
    }
}

#[test]
fn learn_enables_meantone_from_a_12tet_triad() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let backend = RecordingBackend::default();
    state.learn_active = true;
    // Plain 12-TET C-E-G pins a 700¢ fifth and a 400¢ third; since
    // 400 = 4·700 − 2400 this triad IS a meantone.
    hold_chord(&mut state, &[(60, 0.0), (64, 0.0), (67, 0.0)]);
    learn_step(&mut state, &backend);
    assert!(state.view.meantone, "a 12-TET triad should engage meantone");
}

#[test]
fn learn_disables_meantone_from_a_just_triad() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let backend = RecordingBackend::default();
    state.learn_active = true;
    state.view.meantone = true; // start engaged
    // C + a JUST major third (386.31¢) + G. The just third sits a full
    // syntonic comma below four fifths, so this is not a meantone.
    let just_offset = lattice_core::tuning::FIVE_JUST - 400.0;
    hold_chord(&mut state, &[(60, 0.0), (64, just_offset), (67, 0.0)]);
    learn_step(&mut state, &backend);
    assert!(!state.view.meantone, "a just third should release meantone");
}

#[test]
fn learn_leaves_meantone_unchanged_without_a_third() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let backend = RecordingBackend::default();
    state.learn_active = true;
    state.view.meantone = true;
    // A bare fifth fixes no third, so the meantone flag is left alone.
    hold_chord(&mut state, &[(60, 0.0), (67, 0.0)]);
    learn_step(&mut state, &backend);
    assert!(state.view.meantone, "a bare fifth shouldn't change the flag");
}

#[test]
fn audio_spectrum_shows_while_flowing_and_hides_after() {
    let mut spectrum = AudioSpectrum::default();
    let config = SpectrumConfig::default();
    assert!(spectrum.display(0.0, &config).is_none(), "no audio yet");

    // A 440 Hz sine, long enough to fill the analysis window.
    let sine: Vec<f32> = (0..9_000)
        .map(|i| 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin())
        .collect();
    spectrum.push_samples(&sine, 48_000.0, 1.0);
    let (levels, _peaks) = spectrum.display(1.0, &config).expect("audio is flowing");
    let peak = levels
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, _)| i as i32)
        .unwrap();
    // A4 is MIDI 69; its bucket scales with the axis resolution.
    let a4 = ((69.0 - lattice_core::spectrum::SPECTRUM_MIN_MIDI)
        * lattice_core::spectrum::BINS_PER_SEMITONE as f32) as i32;
    assert!((peak - a4).abs() <= 1, "440 Hz should peak at A4 (bucket {a4}), got {peak}");

    // Once samples stop, the curve hides instead of freezing.
    assert!(spectrum.display(1.0 + AudioSpectrum::HOLD_SECONDS + 0.1, &config).is_none());
}

#[test]
fn spectrum_config_round_trips_through_persist() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.spectrum_config.show_audio = true;
    state.spectrum_config.floor_db = -48.0;
    state.spectrum_config.window = SpectrumWindow::Precise;
    state.spectrum_config.low_octave = 1;
    state.spectrum_config.show_spectrogram = true;
    state.spectrum_config.spectrogram_color = crate::SpectrogramColor::Aurora;
    state.spectrum_config.spectrogram_opacity = 0.5;
    state.spectrum_config.spectrogram_smoothing = 0.6;
    state.spectrum_config.roll_outline_width = 2.5;
    let saved = state.save_persist();

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&saved);
    assert!(restored.spectrum_config.show_audio);
    assert_eq!(restored.spectrum_config.floor_db, -48.0);
    assert_eq!(restored.spectrum_config.window, SpectrumWindow::Precise);
    assert_eq!(restored.spectrum_config.low_octave, 1);
    assert!(restored.spectrum_config.show_spectrogram);
    assert_eq!(restored.spectrum_config.spectrogram_color, crate::SpectrogramColor::Aurora);
    assert_eq!(restored.spectrum_config.spectrogram_opacity, 0.5);
    assert_eq!(restored.spectrum_config.spectrogram_smoothing, 0.6);
    assert_eq!(restored.spectrum_config.roll_outline_width, 2.5);
}

/// Every blob saved before the spectrogram existed is missing the field, and
/// plain `#[serde(default)]` answers `false` for it — so the feature arrived
/// switched off for every existing project while a fresh install got it on.
/// The two have to agree.
#[test]
fn a_persist_blob_predating_the_spectrogram_loads_with_it_on() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.spectrum_config.show_spectrogram = false;
    let saved = state.save_persist();
    let old = saved.replace("show_spectrogram:false,", "");
    assert_ne!(old, saved, "the field must have been there to strip");

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&old);
    assert!(
        restored.spectrum_config.show_spectrogram,
        "a missing field must fall back to the struct's own default, not bool::default()"
    );
    // An explicit `false` is a choice, not an absence, and still round-trips.
    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&saved);
    assert!(!restored.spectrum_config.show_spectrogram);
}

#[test]
fn spectrogram_history_stays_bounded() {
    let bins = [0.0f32; lattice_core::spectrum::SPECTRUM_BINS];

    // Age trim: columns older than HISTORY_SECONDS before the newest go.
    let mut spec = AudioSpectrum::default();
    for i in 0..300 {
        spec.push_history(i as f64, bins);
    }
    let cutoff = 299.0 - AudioSpectrum::HISTORY_SECONDS;
    assert!(spec.history().front().unwrap().time >= cutoff, "old columns dropped");
    assert_eq!(spec.history().back().unwrap().time, 299.0, "newest kept");

    // Count cap holds even when every column shares one timestamp (so the
    // age trim never fires) — the backstop against an unbounded ring.
    let mut spec = AudioSpectrum::default();
    for _ in 0..(AudioSpectrum::HISTORY_MAX + 50) {
        spec.push_history(0.0, bins);
    }
    assert!(spec.history().len() <= AudioSpectrum::HISTORY_MAX, "count capped");

    spec.clear_history();
    assert!(spec.history().is_empty());
}

#[test]
fn whole_song_precompute_lays_the_take_out_deterministically() {
    use lattice_core::spectrum::{midi_to_hz, BINS_PER_SEMITONE, SPECTRUM_BINS, SPECTRUM_MIN_MIDI};
    let sr = 48_000.0f32;
    let seconds = 2.0;
    let n = (sr as f64 * seconds) as usize;
    // A steady A4 (MIDI 69) across the whole buffer.
    let freq = midi_to_hz(69.0);
    let samples: Vec<f32> =
        (0..n).map(|i| 0.8 * (std::f32::consts::TAU * freq * i as f32 / sr).sin()).collect();
    let cfg = SpectrumConfig::default();

    let ws = WholeSong::precompute(&samples, sr, 0.0, 0.0, seconds, &cfg);
    assert_eq!(ws.span, seconds);
    assert_eq!(ws.start, 0.0);
    assert!(ws.columns.len() > 10, "a 2 s take yields many columns, got {}", ws.columns.len());

    // Columns are in take time, strictly increasing, inside the take.
    let mut prev = -1.0;
    for c in &ws.columns {
        assert!(c.time > prev, "columns are time-ordered");
        assert!(c.time > 0.0 && c.time <= seconds + 0.1, "column time {} in range", c.time);
        prev = c.time;
    }

    // A steady tone lands its energy at A4's bin.
    let a4 = ((69.0 - SPECTRUM_MIN_MIDI) * BINS_PER_SEMITONE as f32).round() as usize;
    let mid = &ws.columns[ws.columns.len() / 2];
    let peak = (0..SPECTRUM_BINS).max_by(|&a, &b| mid.power[a].total_cmp(&mid.power[b])).unwrap();
    assert!(peak.abs_diff(a4) <= 1, "peak bin {peak} should be A4 (bin {a4})");

    // `time_origin` shifts every column onto the take's timeline.
    let shifted = WholeSong::precompute(&samples, sr, 5.0, 0.0, seconds, &cfg);
    assert!(
        (shifted.columns[0].time - ws.columns[0].time - 5.0).abs() < 1e-6,
        "time_origin offsets the columns"
    );

    // Pure: same inputs in, byte-identical columns out (the render leans on
    // this for reproducibility).
    let again = WholeSong::precompute(&samples, sr, 0.0, 0.0, seconds, &cfg);
    assert_eq!(ws.columns.len(), again.columns.len());
    for (a, b) in ws.columns.iter().zip(&again.columns) {
        assert_eq!(a.time, b.time);
        assert_eq!(a.power, b.power, "precompute is deterministic");
    }
}

/// Every text drawn by one pass over a closure, as (rect, text). The halo
/// stamps each string many times over, so callers fold the stamps of one
/// piece back together into the box that piece occupies.
fn drawn_texts(draw: impl Fn(&egui::Painter)) -> Vec<(egui::Rect, String)> {
    let ctx = egui::Context::default();
    theme::apply_theme(&ctx); // the real Iosevka metrics, not egui's fallback
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
    let output = ctx.run_ui(
        egui::RawInput { screen_rect: Some(screen), ..Default::default() },
        |ui| draw(ui.painter()),
    );
    output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => Some((
                egui::Rect::from_min_size(text.pos, text.galley.size()),
                text.galley.text().to_owned(),
            )),
            _ => None,
        })
        .collect()
}

/// The box one piece of text occupies, halo stamps and all.
fn text_box(texts: &[(egui::Rect, String)], want: &str) -> egui::Rect {
    texts
        .iter()
        .filter(|(_, t)| t == want)
        .map(|(r, _)| *r)
        .reduce(|a, b| a.union(b))
        .unwrap_or_else(|| panic!("no {want:?} drawn, got {texts:?}"))
}

/// The lattice's note labels stack the accidental over the comma mark in one
/// column after the letter, so a name deep in the lattice stays narrow. The
/// whole name still has to sit centered on its node.
#[test]
fn note_label_stacks_the_marks_and_stays_centered_on_the_node() {
    let anchor = egui::pos2(200.0, 200.0);
    let name = lattice_core::NoteName { letter: 'C', sharps: 5, syntonic_commas: 4 };
    let texts = drawn_texts(|painter| {
        panes::lattice::draw_stacked_name(
            painter,
            anchor,
            name,
            egui::Color32::WHITE,
            egui::Color32::BLACK,
            1.0,
        );
    });

    // Counted marks, not five sharps and four pluses spelled out.
    let letter = text_box(&texts, "C");
    let accidental = text_box(&texts, "\u{266F}5");
    let comma = text_box(&texts, "+4");

    // One column, beginning where the letter ends. (Every box here is grown
    // by the halo's rim, so the two edges meet to within that much.)
    const HALO: f32 = 2.0;
    assert!(
        (accidental.left() - letter.right()).abs() <= 2.0 * HALO,
        "marks should follow the letter ({accidental:?} after {letter:?})"
    );
    assert!((accidental.left() - comma.left()).abs() < 0.5, "marks share a column");
    // Superscript over subscript, straddling the letter's own line.
    assert!(accidental.center().y < letter.center().y, "the accidental rides high");
    assert!(comma.center().y > letter.center().y, "the comma sits low");
    // Marks are subordinate to the letter, not the same weight...
    assert!(accidental.height() < letter.height(), "marks are the smaller size");
    // ...and neither stands proud of it: the stacked pair has to stay inside
    // the letter's own height, or the label reads as two lines, not one name.
    assert!(
        accidental.top() >= letter.top() - 0.01 && comma.bottom() <= letter.bottom() + 0.01,
        "marks should not overhang the letter (acc {accidental:?}, comma {comma:?}, \
         letter {letter:?})"
    );

    // The name as a whole straddles the node it labels. (The halo is
    // symmetric, so it grows the box evenly and does not shift the center.)
    let name_box = letter.union(accidental).union(comma);
    assert!(
        (name_box.center().x - anchor.x).abs() < 0.5,
        "name should center on the node ({name_box:?} vs {anchor:?})"
    );
    // ...and stays about as wide as two letters, which is the whole point of
    // counting the marks rather than repeating them.
    assert!(
        name_box.width() < letter.width() * 2.5,
        "a deep name should still fit a node, got {}",
        name_box.width()
    );
}

/// A plain name has no marks to stack -- nothing extra is drawn, and the
/// letter alone centers on the node.
#[test]
fn a_natural_note_label_is_just_the_letter() {
    let anchor = egui::pos2(200.0, 200.0);
    let name = lattice_core::NoteName { letter: 'G', sharps: 0, syntonic_commas: 0 };
    let texts = drawn_texts(|painter| {
        panes::lattice::draw_stacked_name(
            painter,
            anchor,
            name,
            egui::Color32::WHITE,
            egui::Color32::BLACK,
            1.0,
        );
    });
    assert!(texts.iter().all(|(_, t)| t == "G"), "only the letter: {texts:?}");
    assert!((text_box(&texts, "G").center().x - anchor.x).abs() < 0.5);
}


/// The cents readout hangs off the note name's GLYPHS, not its galley box --
/// a monospace line box carries several pixels of leading below the letter,
/// and spacing box-to-box left the readout visibly adrift from the name it
/// belongs to. Drives the whole lattice pane, so it pins what is drawn.
#[test]
fn the_cents_readout_sits_right_under_the_note_name() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let backend = RecordingBackend::default();
    state.view.show_labels = true;
    state.view.show_cents = true;
    // Middle C: the origin node, which the default camera looks straight at.
    state.tracker.handle_event(lattice_core::NoteEvent {
        time: 0.0,
        channel: 0,
        note: 60,
        kind: lattice_core::NoteEventKind::On { velocity: 1.0 },
    });

    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 800.0));
    let output = ctx.run_ui(
        egui::RawInput { screen_rect: Some(screen), time: Some(0.0), ..Default::default() },
        |ui| root_ui(ui, &mut state, &backend, 0.0),
    );

    // A held note lights every node of its pitch class, so each piece of text
    // turns up once per lit node -- and once per halo stamp on top of that.
    // Cluster the stamps back into the pieces they were drawn as, keeping the
    // ink each covers on screen, which is what the eye actually reads.
    let mut names = Vec::new();
    let mut cents = Vec::new();
    for clipped in &output.shapes {
        let egui::Shape::Text(text) = &clipped.shape else { continue };
        // Sort by the label's own type sizes, which nothing else in the dock
        // shares. Not by the text: one pitch class is spelled several ways
        // across the lattice (C, B\u{266F}, D\u{266D}\u{266D}), and every node
        // lit by the held note draws its own name.
        let Some(size) = text.galley.job.sections.first().map(|s| s.format.font_id.size) else {
            continue;
        };
        let pieces = if size == panes::lattice::NAME_SIZE || size == panes::lattice::MARK_SIZE {
            // Letter and marks together: the readout has to clear the comma,
            // which hangs lower than the letter does.
            &mut names
        } else if size == panes::lattice::CENTS_SIZE {
            &mut cents
        } else {
            continue;
        };
        let ink = text.galley.mesh_bounds.translate(text.pos.to_vec2());
        match pieces.iter_mut().find(|seen: &&mut egui::Rect| seen.intersects(ink)) {
            Some(seen) => *seen = seen.union(ink),
            None => pieces.push(ink),
        }
    }
    assert!(!names.is_empty() && !cents.is_empty(), "the held C should be labeled");
    // Each cluster is the piece's ink grown by the halo's rim in every
    // direction; take the rim back off to get the glyphs the eye reads.
    const HALO: f32 = 2.0;
    for piece in names.iter_mut().chain(cents.iter_mut()) {
        *piece = piece.shrink(HALO);
    }

    // Every readout belongs to the name directly above it, and sits the
    // intended air below it -- not the wider, font-dependent gap that
    // box-to-box spacing left behind (6px against a 1px constant).
    for readout in &cents {
        let name = names
            .iter()
            // Overlap, not equal centers: on a node whose name carries marks
            // the letter is pushed left to make room for the mark column,
            // while the readout stays centered on the node itself.
            .filter(|n| n.left() < readout.right() && n.right() > readout.left())
            .filter(|n| n.bottom() <= readout.top())
            .min_by(|a, b| (readout.top() - a.bottom()).total_cmp(&(readout.top() - b.bottom())))
            .unwrap_or_else(|| panic!("no name above {readout:?}, of {names:?}"));
        let gap = readout.top() - name.bottom();
        assert!(
            (gap - panes::lattice::CENTS_GAP).abs() <= 1.0,
            "cents should sit CENTS_GAP under the name, got {gap}px of ink-to-ink gap"
        );
    }
}
