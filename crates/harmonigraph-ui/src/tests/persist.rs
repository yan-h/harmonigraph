//! What survives a session: round-trips through [`UiPersist`], the version
//! floor under it, and the migrations for changes made since that floor.

use crate::*;
use crate::state::UI_PERSIST_VERSION;
use harmonigraph_render::wgpu::TextureFormat;

#[test]
fn persist_round_trips_camera_and_view() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.camera.yaw = 1.23;
    state.camera.distance = 42.0;
    state.view.extent_sevens = 3;
    // Non-default values throughout, so the fields prove they
    // round-trip rather than matching the defaults by luck.
    // Radius 0 is the off state; this proves it (and solidity) persist.
    state.view.core_radius = 0.0;
    state.view.core_solidity = 0.4;
    state.view.outer_inner = 0.1;
    state.view.outer_outer = 0.7;
    state.view.idle_marker = harmonigraph_scene::IdleMarker::Dot;
    state.view.idle_radius = 0.31;
    // Melody alone: both marks on is the default, and this test's whole
    // point is that the fields prove they round-trip rather than
    // matching the defaults by luck.
    state.view.mark_melody = true;
    state.view.mark_bass = false;
    // A window that is neither the default nor a whole number of octaves, so
    // the pair proves it round-trips as two continuous ends rather than
    // landing back on something the layout would have produced anyway.
    state.view.octave_low = 27.5;
    state.view.octave_high = 101.25;
    state.view.grid_color = [0.9, 0.1, 0.4, 0.25];
    state.view.grid_thickness = 2.5;
    state.view.grid_inset = 0.0;
    state.view.grid_dashed = true;
    state.view.meantone = true;
    // Off is the non-default here, and the one a project has to keep: the
    // detect would otherwise re-engage the mode the user switched it off for.
    state.view.meantone_auto = false;
    // The septimal comma's pair of switches carries the same way, and set the
    // other way round from the syntonic one's so a blob that crossed them
    // could not pass.
    state.view.marvel = false;
    state.view.marvel_auto = true;
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
    assert_eq!(restored.view.core_radius, 0.0, "off (radius 0) round-trips");
    assert_eq!(restored.view.core_solidity, 0.4);
    assert_eq!(restored.view.outer_inner, 0.1);
    assert_eq!(restored.view.outer_outer, 0.7);
    assert_eq!(restored.view.idle_marker, harmonigraph_scene::IdleMarker::Dot);
    assert_eq!(restored.view.idle_radius, 0.31);
    assert!(restored.view.mark_melody);
    assert!(!restored.view.mark_bass, "bass off round-trips");
    assert_eq!((restored.view.octave_low, restored.view.octave_high), (27.5, 101.25));
    assert_eq!(restored.view.grid_color, [0.9, 0.1, 0.4, 0.25]);
    assert_eq!(restored.view.grid_thickness, 2.5);
    assert_eq!(restored.view.grid_inset, 0.0, "0 (lines to the center) round-trips");
    assert!(restored.view.grid_dashed);
    assert!(restored.view.meantone);
    assert!(!restored.view.meantone_auto, "a switched-off auto-detect round-trips");
    assert!(!restored.view.marvel, "each comma keeps its own mode");
    assert!(restored.view.marvel_auto, "and its own detect");
    assert_eq!(restored.camera_presets.len(), 1);
    assert_eq!(restored.camera_presets[0].name, "reading");
    assert_eq!(restored.camera_presets[0].yaw, 0.7);
}

#[test]
fn a_blob_written_before_the_auto_detect_opts_into_it() {
    // Every project saved before the switch existed carries no key for it,
    // and each one has a tuning that already answers the question: a 12-TET
    // project IS a meantone (400 = 4·700 − 2400), and its E and E- name one
    // pitch whether or not anyone said "meantone". Defaulting the missing
    // key to OFF would leave exactly those projects the only ones the
    // feature never reaches.
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.camera.yaw = 1.23;
    // One key at a time, each checked to have HIT: with three replacements
    // over one blob, a single `assert_ne!` at the end is satisfied by any one
    // of them, and a key that quietly stopped matching (a rename, a space
    // after the colon) would leave its default untested.
    let mut saved = state.save_persist();
    for key in ["meantone_auto:true,", "marvel_auto:true,", "marvel:false,"] {
        let stripped = saved.replace(key, "");
        assert_ne!(stripped, saved, "{key:?} is not in the blob to remove");
        saved = stripped;
    }

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&saved);
    assert!(restored.view.meantone_auto, "a missing key means on");
    // And the septimal comma's keys are newer still, so EVERY project
    // predates them: 12-TET tempers 225/224 out as well (1000 = 2·700 + 2·400
    // − 1200), so the same argument opts them in — off would leave the mode
    // unreachable for every project that already exists.
    assert!(restored.view.marvel_auto, "a missing detect key means on");
    assert!(!restored.view.marvel, "a missing mode key means off, and the detect decides");
    assert_eq!(restored.camera.yaw, 1.23, "rest of the blob still restores");
}

/// The octave wheel was a COUNT of octaves either side of middle C's before it
/// was a pitch window, and a project saved against it has to open on the same
/// picture: `2 * span + 1` octaves, centered on middle C.
///
/// ±5 is the one that cannot be reproduced exactly — it reached half an octave
/// under MIDI 0, which is under the lowest pitch the window can name — so it
/// lands at the floor with the same eleven octaves in it, the bottom one cut
/// off at the seam rather than centered on it. That is a whole octave of the
/// wheel either way, which is why it is worth pinning rather than leaving to
/// whatever the clamp happens to do.
#[test]
fn a_blob_written_against_the_octave_count_opens_on_the_window_it_named() {
    for (span, low, high) in
        [(2u32, 30.0f32, 90.0f32), (3, 18.0, 102.0), (4, 6.0, 114.0), (5, 0.0, 126.0)]
    {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        state.camera.yaw = 1.23;
        // The window keys are what a blob written against the count does NOT
        // carry, so swapping the pair for `octave_span` is exactly that blob.
        let saved = state.save_persist().replace(
            &format!(
                "octave_low:{:?},octave_high:{:?}",
                state.view.octave_low, state.view.octave_high
            ),
            &format!("octave_span:{span}"),
        );
        assert_ne!(saved, state.save_persist(), "replacement must have hit for ±{span}");

        let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
        restored.load_persist(&saved);
        assert_eq!(
            (restored.view.octave_low, restored.view.octave_high),
            (low, high),
            "±{span} names {low}..{high}"
        );
        assert_eq!(restored.camera.yaw, 1.23, "rest of the blob still restores (±{span})");
    }
}

#[test]
fn a_pre_split_melody_bass_blob_loads_as_the_two_flags() {
    // The two marks were one four-way enum before they became the pair of
    // flags they always were. An old blob carries `highlight_extremes` and
    // NEITHER flag, and it writes the variant BARE — so without the
    // load-only shim the token wouldn't parse into an Option and the failed
    // parse would drop the WHOLE persist, camera and layout with it.
    for (token, melody, bass) in [
        ("Off", false, false),
        ("Melody", true, false),
        ("Bass", false, true),
        ("Both", true, true),
    ] {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        state.camera.yaw = 1.23;
        state.view.mark_melody = false;
        state.view.mark_bass = false;
        let saved = state.save_persist().replace(
            "mark_melody:false,mark_bass:false",
            &format!("highlight_extremes:{token}"),
        );
        assert_ne!(saved, state.save_persist(), "replacement must have hit for {token}");

        let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
        restored.load_persist(&saved);
        assert_eq!(restored.view.mark_melody, melody, "{token} -> melody");
        assert_eq!(restored.view.mark_bass, bass, "{token} -> bass");
        assert_eq!(restored.camera.yaw, 1.23, "rest of the blob still restores ({token})");
    }
}

#[test]
fn a_render_frame_saved_as_stacked_loads_as_the_side_it_meant() {
    // The frame's arrangement was `stacked: bool` before it became four named
    // sides: `true` put the lattice above the spectral pane, `false` to its
    // left. Old blobs carry the flag and no `lattice`, and so does the
    // `ui_state` inside every take recorded then — which is why both doors
    // into the blob have to fold it, or a take framed stacked re-renders side
    // by side.
    for (flag, side) in [(true, LatticeSide::Top), (false, LatticeSide::Left)] {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        state.camera.yaw = 1.23;
        // A side the migration can't reach by accident, so a shim that failed
        // to fire is visible rather than looking like the default.
        state.take.render_config.frame.lattice = LatticeSide::Right;
        state.take.render_config.frame.split = 0.42;
        let saved = state.save_persist().replace("lattice:Right", &format!("stacked:{flag}"));
        assert_ne!(saved, state.save_persist(), "replacement must have hit for {flag}");

        let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
        restored.load_persist(&saved);
        assert_eq!(restored.take.render_config.frame.lattice, side, "stacked:{flag}");
        assert_eq!(restored.take.render_config.frame.split, 0.42, "the rest of the frame survives");
        assert_eq!(restored.camera.yaw, 1.23, "rest of the blob still restores");

        // The offline renderer's own door into the blob.
        let render = crate::render_config_from_persist(&saved).expect("still parses");
        assert_eq!(render.frame.lattice, side, "stacked:{flag} through render_config_from_persist");
    }
}

/// The flag is load-only: a saved frame names its side and says nothing about
/// `stacked`, so a blob written now cannot be read back as a migration.
#[test]
fn a_saved_render_frame_carries_the_side_and_not_the_old_flag() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.take.render_config.frame.lattice = LatticeSide::Bottom;
    let saved = state.save_persist();
    assert!(saved.contains("lattice:Bottom"), "the side is what gets written");
    assert!(!saved.contains("stacked:"), "the shim must never be written back");

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&saved);
    assert_eq!(restored.take.render_config.frame.lattice, LatticeSide::Bottom);
}

/// A saved `--size` in the Options text becomes the Resolution control, and
/// stops overriding the aspect it was saved beside.
///
/// This is the whole point of the migration: `--size 1920x1080` was the
/// Options DEFAULT, so every project carries it, and it was read after the
/// Aspect row — a 9:16 frame previewed tall and rendered 16:9. Lifting the
/// short edge out keeps the size the user chose and hands the shape back to
/// the frame.
#[test]
fn a_size_saved_in_the_options_text_loads_as_the_resolution() {
    let mut config = crate::RenderConfig {
        legacy_extra_args: "--size 1920x1080".into(),
        frame: crate::RenderFrame { aspect_w: 9, aspect_h: 16, ..Default::default() },
        ..Default::default()
    };
    config.migrate_legacy();
    assert_eq!(config.short_edge, 1080, "the short edge is what the flag meant");
    assert_eq!(config.legacy_extra_args, "", "and the flag itself is gone");
    // The frame now decides the shape, which it never got to before.
    assert_eq!(config.frame.pixels(config.short_edge), [1080, 1920]);
}

/// The migration reads sizes the way the renderer does, and leaves everything
/// it does not understand for the renderer to answer for.
#[test]
fn lifting_a_size_out_of_the_options_text_leaves_the_rest_of_it_alone() {
    let migrated = |args: &str| {
        let mut config =
            crate::RenderConfig { legacy_extra_args: args.into(), ..Default::default() };
        config.migrate_legacy();
        (config.short_edge, config.legacy_extra_args)
    };

    // Other flags keep their order and their spacing is normalized.
    assert_eq!(
        migrated("--fps 30 --size 3840x2160 --layout stacked"),
        (2160, "--fps 30 --layout stacked".to_string())
    );
    // The short edge, not the width: a portrait size means a portrait render.
    assert_eq!(migrated("--size 1080x1920"), (1080, String::new()));
    // The renderer keeps the LAST of a repeated flag, so the migration has to
    // agree with it or it would change the picture it claims to preserve.
    assert_eq!(migrated("--size 1920x1080 --size 2560x1440"), (1440, String::new()));
    // The renderer's other spellings.
    assert_eq!(migrated("-s 3840X2160"), (2160, String::new()));
    // No size to lift: the control keeps its default and the text is untouched.
    assert_eq!(migrated("--fps 30"), (1080, "--fps 30".to_string()));
    // An unparseable size stays put, for the renderer to reject out loud
    // rather than being silently eaten here.
    assert_eq!(migrated("--size wide"), (1080, "--size wide".to_string()));
}

/// The lift happens on the real load path, not just when called directly.
///
/// The blob is doctored rather than saved from state, because the field is
/// `skip_serializing` and a round trip through `save_persist` could no longer
/// produce one — which is the point of the shim: what it reads is a blob some
/// EARLIER build wrote, and this build has no way to write another.
#[test]
fn a_loaded_blob_has_its_options_size_lifted() {
    let state = SharedState::new(TextureFormat::Bgra8Unorm);
    let saved = state.save_persist();
    assert!(!saved.contains("extra_args"), "the shim must never be written back");
    // Where an Options field sat in the render config, beside the control it
    // used to outrank.
    let old = saved.replace("short_edge:", "extra_args:\"--size 2560x1440\",short_edge:");
    assert_ne!(old, saved, "the injection must have hit");

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&old);
    assert_eq!(restored.take.render_config.short_edge, 1440, "the size became the Resolution");
    assert_eq!(restored.take.render_config.legacy_extra_args, "", "and the text was consumed");
}

#[test]
fn corrupt_persist_is_ignored() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let default_distance = state.camera.distance;
    state.load_persist("not json at all");
    assert_eq!(state.camera.distance, default_distance);
}

#[test]
fn spectrum_config_round_trips_through_persist() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.spectrum_config.floor_db = -48.0;
    state.spectrum_config.ceiling_db = -12.0;
    state.spectrum_config.window = SpectrumWindow::Precise;
    state.spectrum_config.low_midi = 40.5;
    state.spectrum_config.show_spectrogram = true;
    state.spectrum_config.spectrogram_color = crate::SpectrogramColor::Aurora;
    let saved = state.save_persist();

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&saved);
    assert_eq!(restored.spectrum_config.floor_db, -48.0);
    assert_eq!(restored.spectrum_config.ceiling_db, -12.0);
    assert_eq!(restored.spectrum_config.window, SpectrumWindow::Precise);
    // A range off the C boundaries survives, which the octave pair could not
    // have expressed at all.
    assert_eq!(restored.spectrum_config.low_midi, 40.5);
    assert!(restored.spectrum_config.show_spectrogram);
    assert_eq!(restored.spectrum_config.spectrogram_color, crate::SpectrogramColor::Aurora);
}

/// A range saved while the axis ran MIDI 12..132 (16 Hz to 16.7 kHz) starts
/// below the 20 Hz floor the axis has now. Drawing it would leave a band with
/// no buckets behind it, so loading fits the range to the axis that exists —
/// and only where it has to: 132 is still a pitch this axis covers.
#[test]
fn a_pitch_range_off_the_current_axis_is_pulled_back_onto_it() {
    use harmonigraph_core::spectrum::{SPECTRUM_MAX_MIDI, SPECTRUM_MIN_MIDI};
    let restore = |low: &str, high: &str| {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        state.spectrum_config.low_midi = 60.0;
        state.spectrum_config.high_midi = 72.0;
        let saved = state
            .save_persist()
            .replace("low_midi:60.0", &format!("low_midi:{low}"))
            .replace("high_midi:72.0", &format!("high_midi:{high}"));
        let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
        restored.load_persist(&saved);
        (restored.spectrum_config.low_midi, restored.spectrum_config.high_midi)
    };

    let (low, high) = restore("12.0", "132.0");
    assert_eq!(low, SPECTRUM_MIN_MIDI, "below the floor, so pulled up to it");
    assert_eq!(high, 132.0, "inside the axis, so left exactly where it was");

    // A hand-edited blob can reach past the ceiling too.
    let (_, high) = restore("40.0", "200.0");
    assert_eq!(high, SPECTRUM_MAX_MIDI);
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

/// A field that has since been REMOVED must not take the whole blob down with
/// it. A blob that fails to parse loses the entire UI state, not just the stale
/// key — so every settings removal rides on serde ignoring what it has no field
/// for, and this is where that is held.
///
/// `spectrogram_fine_levels` existed only while the heatmap's stored precision
/// was being judged by eye; the other four are the heatmap's opacity, contrast
/// and private dB window, which every project saved before they were dropped
/// still carries.
#[test]
fn a_persist_blob_carrying_a_since_removed_field_still_loads() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.view.extent_sevens = 3;
    let saved = state.save_persist();
    // Put the departed fields back, exactly as those builds wrote them.
    let gone = "spectrogram_fine_levels:true,spectrogram_opacity:0.85,\
                spectrogram_own_range:true,spectrogram_floor_db:-60.0,\
                spectrogram_ceiling_db:-20.0,spectrogram_gamma:1.6,";
    let stale = saved.replace("spectrogram_color:", &format!("{gone}spectrogram_color:"));
    assert_ne!(stale, saved, "the anchor field must have been there to splice onto");

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&stale);
    assert_eq!(restored.view.extent_sevens, 3, "an unknown field must not sink the blob");
}

/// The mirror of the case above: a field the blob is MISSING must not sink it
/// either, and must come back as what a fresh install has rather than as a bare
/// `0`/`false`.
///
/// `smoothing`, `window` and `floor_db` are the three that make this worth
/// pinning. Every other field of [`SpectrumConfig`](crate::SpectrumConfig)
/// named a fallback of its own; these three named none, so a blob without one
/// failed to parse — and a blob that fails to parse loses the WHOLE UI state,
/// not the one key. Dropping `smoothing` cost the camera, the dock and the view
/// along with it. The struct's container-level `default` is what closes that:
/// every field falls back to `impl Default`'s value, so a missing key costs
/// only itself.
///
/// Pinned per field rather than once, because the hazard is per field: nothing
/// at a declaration says whether it has a fallback, so the next field added is
/// covered silently and the next one REMOVED is the one that would sink a saved
/// project.
#[test]
fn a_persist_blob_missing_a_spectrum_field_keeps_the_rest_of_the_blob() {
    let fresh = crate::SpectrumConfig::default();
    for key in [
        format!("smoothing:{:?},", fresh.smoothing),
        format!("floor_db:{:?},", fresh.floor_db),
        format!("window:{:?},", fresh.window),
    ] {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        // A non-default elsewhere in the blob, so "the blob survived" is
        // distinguishable from "it sank and everything reverted".
        state.view.extent_sevens = 3;
        let saved = state.save_persist();
        let without = saved.replacen(key.as_str(), "", 1);
        assert_ne!(without, saved, "{key:?} must be in the blob to drop");

        let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
        restored.load_persist(&without);
        assert_eq!(
            restored.view.extent_sevens, 3,
            "dropping {key:?} must cost that key alone, not the whole blob",
        );
        assert_eq!(
            restored.spectrum_config, fresh,
            "and the config it belongs to must load at the fresh-install values",
        );
    }
}

/// The blob's top-level keys are [`UiPersist`]'s, whatever shape the state
/// they are read out of has.
///
/// `save_persist` copies field by field out of [`SharedState`] into a struct
/// of its own, so how the state groups those fields is not a persistence
/// question — which is the only reason regrouping them is safe. This is what
/// says so: a grouping that reached the blob renames or nests a key here, and
/// a key that moves is a saved project that loads at defaults.
///
/// The ORDER too, not just the set: `load_persist` is order-insensitive, but a
/// reordered blob is a diff no reviewer can tell from a reshaped one.
#[test]
fn the_persist_blob_carries_exactly_these_top_level_keys() {
    // UiPersist's fields, in declaration order.
    const KEYS: &[&str] = &[
        "version",
        "dock",
        "folds",
        "camera",
        "view",
        "camera_presets",
        "spectrum",
        "render",
        "fps_cap",
        "ui_scale",
    ];

    let saved = SharedState::new(TextureFormat::Bgra8Unorm).save_persist();
    let keys: Vec<String> = top_level_pairs(&saved).into_iter().map(|(key, _)| key).collect();
    assert_eq!(keys, KEYS, "the persist blob's top-level keys have moved");
}

/// Dropping the render settings from a blob costs the render settings alone.
///
/// The container-level `#[serde(default)]` on [`RenderConfig`] covers a key
/// missing from INSIDE the section (the case
/// `a_persist_blob_missing_a_spectrum_field_keeps_the_rest_of_the_blob` pins
/// one layer down); this covers the whole section being absent.
///
/// No blob on disk is in that shape, and the floor is why: `render` entered
/// [`UiPersist`] before [`UI_PERSIST_VERSION`] last moved, so every blob
/// written without it is below the floor and refused whole. What is left to
/// pin is the `#[serde(default)]` on the field itself — a hand-edited
/// `app.ron` can be missing it, and the next writer of this blob has no other
/// statement that dropping one section must not sink the other nine.
///
/// Both doors, because they answer separately: `load_persist` for the rest of
/// the settings, and `render_config_from_persist` for the frame the offline
/// renderer composes at. The renderer does have a fallback behind that door
/// (`main`'s `unwrap_or_default`), so what this holds is the two doors
/// AGREEING about one blob — the property
/// `both_doors_into_a_blob_agree_about_the_version_floor` holds at the version.
#[test]
fn a_persist_blob_missing_the_render_section_keeps_the_rest_of_the_blob() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    // Non-defaults on both sides of the drop, so "the blob survived" is
    // distinguishable from "it sank and everything reverted".
    state.view.extent_sevens = 3;
    state.take.render_config.lead_in = 2.5;
    let saved = state.save_persist();

    let kept: Vec<String> = top_level_pairs(&saved)
        .into_iter()
        .filter(|(key, _)| key != "render")
        .map(|(_, text)| text)
        .collect();
    let without = format!("({})", kept.join(","));
    assert_ne!(without, saved, "the render section must be in the blob to drop");

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    // Loaded OVER a config that is not the fresh one, which is what makes the
    // assertion below mean anything: a fresh state already holds
    // `RenderConfig::default()`, so a load that skipped the section entirely
    // would satisfy "it is at the fresh-install values" without doing it.
    restored.take.render_config.lead_in = 4.0;
    restored.take.render_config.short_edge = 2160;
    restored.load_persist(&without);
    assert_eq!(
        restored.view.extent_sevens, 3,
        "dropping the render settings must cost them alone, not the whole blob",
    );
    // Serialized rather than field by field: RenderConfig has no PartialEq, and
    // the point is that the WHOLE section is at fresh-install values.
    let fresh = ron::to_string(&crate::RenderConfig::default()).expect("a config serializes");
    assert_eq!(
        ron::to_string(&restored.take.render_config).expect("a config serializes"),
        fresh,
        "and the settings themselves must load at the fresh-install values",
    );

    let door = crate::render_config_from_persist(&without).expect("the blob still parses");
    assert_eq!(
        ron::to_string(&door).expect("a config serializes"),
        fresh,
        "the renderer's door must answer the same, rather than refusing the blob",
    );
}

#[test]
fn persist_round_trips_the_frame_rate_cap() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    // Not one of the button values, so it proves the number round-trips
    // rather than being re-derived from a default.
    state.fps_cap = Some(45.0);

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&state.save_persist());
    assert_eq!(restored.fps_cap, Some(45.0));
}

#[test]
fn pre_cap_persist_blobs_load_as_uncapped() {
    // The cap was added after these blobs were written; dropping the field
    // must not fail the parse, which would silently discard the WHOLE
    // persist (layout, camera, every view setting) rather than one setting.
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.fps_cap = Some(30.0);
    state.view.extent_sevens = 3;
    let saved = state.save_persist();
    let stripped = saved.replace(",fps_cap:Some(30.0)", "");
    assert_ne!(stripped, saved, "the field removal must have hit");

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&stripped);
    assert_eq!(restored.fps_cap, None, "a missing cap reads as uncapped");
    assert_eq!(restored.view.extent_sevens, 3, "the rest of the blob must survive");
}

/// Orientations that no longer exist must still PARSE, and land where the
/// setting they named would put the picture.
///
/// The same threat the palette aliases below answer: a blob naming a variant
/// the enum has dropped fails to parse, and takes the WHOLE persist with it
/// rather than the one setting. `Horizontal` and `Vertical` were the names
/// while they meant the pitch axis; `Auto` picked a layout off the pane's
/// shape and has no successor, so it lands on the default the pane opens at.
#[test]
fn removed_spectral_orientations_load_as_their_successors() {
    use crate::SpectralOrientation;
    for (removed, want) in [
        ("Horizontal", SpectralOrientation::Left),
        ("Vertical", SpectralOrientation::Top),
        ("Auto", SpectralOrientation::Left),
    ] {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = SpectralOrientation::Bottom;
        state.view.extent_sevens = 3;
        let saved = state
            .save_persist()
            .replace("orientation:Bottom", &format!("orientation:{removed}"));
        assert_ne!(saved, state.save_persist(), "replacement must have hit for {removed}");

        let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
        restored.load_persist(&saved);
        assert_eq!(restored.spectrum_config.orientation, want, "{removed} loaded elsewhere");
        assert_eq!(restored.view.extent_sevens, 3, "the rest of the blob must survive");
    }
}

/// Palettes that no longer exist must still PARSE. Serde aliases fold them
/// onto Magma, the nearest surviving ramp; without them the failed parse
/// would drop the whole persist — layout, camera and every view setting with
/// it — not just the palette. Injected as strings, since the enum can no
/// longer name them.
#[test]
fn removed_spectrogram_palettes_load_as_magma() {
    for removed in ["Heat", "Pitch", "Paper"] {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        state.spectrum_config.spectrogram_color = crate::SpectrogramColor::Aurora;
        state.view.extent_sevens = 3;
        let saved = state
            .save_persist()
            .replace("spectrogram_color:Aurora", &format!("spectrogram_color:{removed}"));
        assert_ne!(saved, state.save_persist(), "replacement must have hit for {removed}");

        let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
        restored.load_persist(&saved);
        assert_eq!(
            restored.spectrum_config.spectrogram_color,
            crate::SpectrogramColor::Magma,
            "{removed} should fold onto the nearest surviving ramp",
        );
        assert_eq!(restored.view.extent_sevens, 3, "the rest of the blob must survive");
    }
}

/// A key this build has RETIRED does not cost the blob it sits in.
///
/// `load_persist` takes the whole `UiPersist` or nothing (`if let Ok(persist)`),
/// so a field that fails to parse does not degrade — it silently discards the
/// dock, the camera and the entire `ViewConfig` along with itself, and the
/// project opens on defaults with no error anywhere. Retiring a setting is
/// therefore a persistence change, and this is the guard on it: `roll_gap` went
/// with the Gap feature and `roll_color` with the roll's Color row, and every
/// project saved before each still carries its key.
///
/// Both SHAPES of value, because they are not the same risk. A retired key
/// holding a NUMBER is skipped by any parser worth the name. One holding a bare
/// identifier is the shape that has actually cost a blob here — a
/// `SpectrogramColor` naming a palette this build no longer has takes the whole
/// persist with it, which is why the deleted palettes keep serde aliases. What
/// separates the two is that retiring a FIELD is safe where retiring a VARIANT
/// is not, and a test that only ever splices a number cannot tell them apart.
#[test]
fn a_retired_setting_does_not_discard_the_blob_it_was_saved_in() {
    for retired in ["roll_gap:2.5,", "roll_color:Pitch,"] {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        state.spectrum_config.roll_thickness = 1.75;
        state.spectrum_config.low_midi = 40.5;
        let saved = state.save_persist();
        // A blob from before the retirement: the key spliced back where it sat.
        let old = saved.replacen("roll_thickness:", &format!("{retired}roll_thickness:"), 1);
        assert_ne!(old, saved, "the {retired} splice must land for this to test anything");

        let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
        restored.load_persist(&old);
        assert_eq!(
            restored.spectrum_config.roll_thickness, 1.75,
            "the blob carrying {retired} survived",
        );
        assert_eq!(restored.spectrum_config.low_midi, 40.5);
    }
}

/// A blob saved before the control existed loads at the design size. `f32`'s
/// own serde default is 0.0 — a scale of nothing, and every one of those
/// projects.
#[test]
fn a_blob_without_a_ui_scale_loads_at_the_design_size() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.ui_scale = 0.75;
    let saved = state.save_persist();
    let mut back = SharedState::new(TextureFormat::Bgra8Unorm);
    back.load_persist(&saved);
    assert_eq!(back.ui_scale, 0.75, "the scale did not round-trip");

    // The same blob with the field taken back out, which is what every project
    // saved before this looks like.
    let stripped = saved
        .split(",ui_scale:")
        .next()
        .expect("the blob names the field")
        .to_owned()
        + ")";
    let mut older = SharedState::new(TextureFormat::Bgra8Unorm);
    older.load_persist(&stripped);
    assert_eq!(older.ui_scale, 1.0, "a blob with no scale did not load at the design size");
}

/// An out-of-range scale — only a hand-edited blob can produce one — is
/// clamped rather than drawn at.
#[test]
fn an_impossible_ui_scale_is_clamped() {
    let ctx = egui::Context::default();
    crate::theme::apply_theme(&ctx);
    // A nonsense value falls back to the design size rather than to the end of
    // the range it points at: an infinity is not a request for the largest
    // chrome available, it is a blob that has lost the number.
    for (asked, expected) in
        [(0.01f32, 0.7f32), (99.0, 1.5), (f32::NAN, 1.0), (f32::INFINITY, 1.0)]
    {
        crate::theme::set_ui_scale(&ctx, asked);
        assert_eq!(
            crate::theme::ui_scale(&ctx),
            expected,
            "a scale of {asked} was taken at face value",
        );
    }
}

/// A non-finite end of the analyzer's pitch range loads at the design range
/// rather than taking the editor down with it.
///
/// `clamp` alone does not catch a NaN — it is its own answer to every
/// comparison — and `f32::clamp` opens with `assert!(min <= max)`. So a NaN
/// `low_midi` survives its own clamp and then becomes the MIN of the next one,
/// which fails that assert: the editor panics on load, and the only trace is
/// the backtrace the host writes to its log.
///
/// A NaN `high_midi` does not panic — it is the `self` of its clamp rather
/// than the bound — and is worse for it: the range stays NaN all the way into
/// `PitchScale`, so the analyzer draws nothing and nothing says why.
///
/// The bars cannot produce either; a hand-edited blob or a corrupted float
/// can. This function already guards its two text scales against exactly this
/// (see [`sane_scale`]) — the pitch range is the half that was left bare.
#[test]
fn a_blob_with_a_nonsense_pitch_range_loads_at_the_design_range() {
    for end in ["low_midi", "high_midi"] {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        // Off the defaults, so the splice has something to name and a range
        // that survives proves it survived rather than matching by luck.
        state.spectrum_config.low_midi = 40.5;
        state.spectrum_config.high_midi = 90.25;
        let saved = state.save_persist();
        let value = if end == "low_midi" { 40.5f32 } else { 90.25f32 };
        let broken = saved.replacen(&format!("{end}:{value:?}"), &format!("{end}:NaN"), 1);
        assert_ne!(broken, saved, "the {end} splice must land for this to test anything");

        let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
        restored.load_persist(&broken);
        let (low, high) = (restored.spectrum_config.low_midi, restored.spectrum_config.high_midi);
        assert!(low.is_finite() && high.is_finite(), "{end}:NaN left the range at {low}..{high}");
        assert!(low < high, "{end}:NaN left the range inverted at {low}..{high}");
        // The end that was NOT broken keeps what the blob said, so a guard
        // cannot pass by resetting the whole range.
        if end == "low_midi" {
            assert_eq!(high, 90.25, "the good end still loads");
        } else {
            assert_eq!(low, 40.5, "the good end still loads");
        }
    }
}

#[test]
fn a_blob_older_than_the_version_floor_is_refused_whole() {
    // Versions below UI_PERSIST_VERSION cannot reach a real editor: the
    // plugin's CLAP/VST3 ids changed three days after the version last moved,
    // so a project old enough to carry one names an identity this binary does
    // not claim. Refusing it here is what lets the migrations for those
    // formats be deleted rather than carried forever.
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.camera.yaw = 1.23;
    state.view.extent_sevens = 3;
    let saved = state.save_persist();
    assert!(saved.contains(&format!("version:{UI_PERSIST_VERSION}")), "saves at the floor");

    let stale = saved.replacen(
        &format!("version:{UI_PERSIST_VERSION}"),
        &format!("version:{}", UI_PERSIST_VERSION - 1),
        1,
    );
    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&stale);
    // Untouched, not partially applied: a refused blob must not leave the
    // camera from one era beside a dock from another.
    let fresh = SharedState::new(TextureFormat::Bgra8Unorm);
    assert_eq!(restored.camera.yaw, fresh.camera.yaw);
    assert_eq!(restored.view.extent_sevens, fresh.view.extent_sevens);

    // And the same blob at the floor still loads, so the test above is
    // measuring the version rather than a blob that was broken anyway.
    let mut current = SharedState::new(TextureFormat::Bgra8Unorm);
    current.load_persist(&saved);
    assert_eq!(current.camera.yaw, 1.23);
    assert_eq!(current.view.extent_sevens, 3);
}

/// The two doors into a take's `ui_state` agree about whether it is loadable.
///
/// The offline renderer reads the SAME blob twice: `render_config_from_persist`
/// for the frame it composes at and the lead-in it starts from, and
/// `load_persist` for the camera, view and spectrum it draws with. A floor on
/// one and not the other renders an old take at its recorded size and aspect —
/// so the output looks honoured — around a lattice nobody dialled in, with the
/// whole-song playhead the take asked for silently off.
///
/// The floor above is argued from plugin identity, and that argument covers one
/// of `load_persist`'s three callers. A `.take` is a file on disk and
/// `harmonigraph-take` refuses only takes from the FUTURE, so an old one opens
/// and hands its `ui_state` straight through; the standalone's `app.ron` has no
/// identity gate either. What keeps this from being reachable today is only
/// that every take on disk is at the current version — which the next bump
/// ends, for all of them at once.
///
/// Costs no shim: `stacked` and the `--size` inside `extra_args` are both
/// NEWER than the floor, so every blob they migrate is at the floor already.
#[test]
fn both_doors_into_a_blob_agree_about_the_version_floor() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    // Values a default cannot produce, so a door that drops the blob is visible
    // rather than looking like it loaded something.
    state.take.render_config.frame.aspect_w = 9;
    state.take.render_config.frame.aspect_h = 16;
    state.take.render_config.playhead = true;
    state.take.render_config.lead_in = 2.5;
    let saved = state.save_persist();

    let stale = saved.replacen(
        &format!("version:{UI_PERSIST_VERSION}"),
        &format!("version:{}", UI_PERSIST_VERSION - 1),
        1,
    );
    assert_ne!(stale, saved, "the version splice must land for this to test anything");

    // The door `load_persist` is: refused, so the render config is the default.
    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&stale);
    let fresh = SharedState::new(TextureFormat::Bgra8Unorm);
    assert_eq!(restored.take.render_config.playhead, fresh.take.render_config.playhead);

    // The door `harmonigraph-offline`'s `main` is. Refusing the blob whole
    // means refusing it here too, so the renderer composes at a default frame
    // it can see rather than a recorded one wrapped around defaults.
    assert!(
        crate::render_config_from_persist(&stale).is_none(),
        "one door honoured a blob the other refused",
    );

    // And a blob AT the floor still comes through both, so this is measuring
    // the version rather than a blob that was broken anyway.
    let at_floor = crate::render_config_from_persist(&saved).expect("the floor still parses");
    assert_eq!(at_floor.frame.aspect_w, 9);
    assert!(at_floor.playhead);
    assert_eq!(at_floor.lead_in, 2.5);
}

/// Loading a project asks the detects afresh, even at a tuning this session
/// has already judged.
///
/// A host can push state into a LIVE editor — Bitwig's undo, a preset change
/// — and the modes that arrive are the incoming project's, so the verdicts
/// reached about the tuning on screen a moment ago say nothing about them. It
/// matters most for the case the serde defaults exist for: a blob written
/// before a comma existed carries that mode off, and only a fresh look turns
/// it on.
#[test]
fn loading_a_project_re_opens_the_comma_verdicts() {
    use harmonigraph_core::Comma;
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    // A blob from before the septimal comma existed: its keys are stripped,
    // so `marvel` defaults off and `marvel_auto` on.
    let saved = state.save_persist().replace("marvel:false,", "").replace("marvel_auto:true,", "");
    assert_ne!(saved, state.save_persist(), "removal must have hit");

    // This session has already judged the tuning it is sitting at.
    state.temper_judged = [Some((0, 0, 0)); Comma::COUNT];
    state.load_persist(&saved);
    assert_eq!(
        state.temper_judged,
        [None; Comma::COUNT],
        "a loaded project must be judged on its own terms",
    );
    assert!(state.view.marvel_auto, "and the missing detect key still opts in");
}



/// [`ViewConfig`](harmonigraph_scene::ViewConfig)'s serde fallbacks and its
/// `impl Default` answer DIFFERENTLY on purpose, and this pins which fields.
///
/// The rule the struct is built on: a serde fallback is what a blob written
/// before that field existed was DRAWN with, so loading an old view does not
/// restyle it, while `impl Default` is the look a fresh view opens in. Where
/// the two disagree the value is written literally in `impl Default`, so
/// retuning the out-of-the-box look never reaches someone's saved project.
///
/// That rule is carried by hand, by two mechanisms that look nothing alike: a
/// named `default_*` fn where the old value has to be stated, and a BARE
/// `#[serde(default)]` where `T::default()` already is the old value —
/// `IdleMarker::Circle` (the classic placeholder look), `TrailMark::Off`, a
/// bloom of 0 from before the chain existed. Nothing at a declaration says
/// which of those two a field is using, or whether anyone chose.
///
/// So this probes it from the outside instead: drop one key at a time from a
/// serialized fresh view and reload, which is exactly what an old blob is.
/// A field whose name is listed below reloads as something OTHER than the
/// fresh-install value — which is the intent for every name currently on it.
///
/// A name APPEARING here means a field just stopped handing old blobs the
/// fresh value; a name LEAVING means a saved view will now be restyled by a
/// change to `impl Default`, which is the failure the whole arrangement
/// exists to prevent. Either way the diff is the review question, and
/// updating this list is the deliberate edit that answers it.
///
/// Deliberately NOT the same call as `SpectrumConfig`'s, which collapsed its
/// per-field fallbacks into one container-level `#[serde(default)]`. That is
/// right there because its fallbacks and its `impl Default` are meant to
/// agree; here they are meant not to.
#[test]
fn the_view_fields_an_old_blob_reloads_differently_are_exactly_these() {
    // In serialization order, which is the order the probe reports them.
    const LEGACY: &[&str] = &[
        "sevens_gutter",
        "core_solidity",
        "core_radius",
        "outer_inner",
        "outer_outer",
        "outer_gap",
        "idle_marker",
        "idle_radius",
        "mark_thickness",
        "grid_thickness",
        "grid_inset",
        "trail_mark",
        "trail_labels",
        "bloom_strength",
    ];
    // These have no fallback at all: drop one and the WHOLE view fails to
    // parse, taking every other field with it. Harmless only because all four
    // predate the persist version floor, so no blob that exists is missing
    // one. The list must not GROW — a new field landing here is the bug
    // `a_persist_blob_missing_a_spectrum_field_keeps_the_rest_of_the_blob`
    // was written for, one layer up.
    const NO_FALLBACK: &[&str] = &["spacing", "extent_threes", "extent_fives", "extent_sevens"];

    let fresh = harmonigraph_scene::ViewConfig::default();
    let full = ron::to_string(&fresh).expect("a view serializes");
    let pairs = top_level_pairs(&full);
    assert!(pairs.len() > 40, "the probe must see the whole struct, got {}", pairs.len());

    let (mut legacy, mut no_fallback) = (Vec::new(), Vec::new());
    for (key, _) in &pairs {
        // The blob rebuilt without this one key IS a blob written before the
        // field existed, which is the case the fallbacks are for.
        let kept: Vec<&str> =
            pairs.iter().filter(|(k, _)| k != key).map(|(_, text)| text.as_str()).collect();
        let without = format!("({})", kept.join(","));
        match ron::from_str::<harmonigraph_scene::ViewConfig>(&without) {
            Err(_) => no_fallback.push(key.as_str()),
            Ok(loaded) => {
                if ron::to_string(&loaded).expect("a view serializes") != full {
                    legacy.push(key.as_str());
                }
            }
        }
    }

    assert_eq!(legacy, LEGACY, "the fields an old blob reloads differently have changed");
    assert_eq!(no_fallback, NO_FALLBACK, "a field without a serde fallback sinks the whole view");
}

/// Split a serialized struct into its top-level `key:value` pairs, as
/// `(key, whole pair)`. Depth-aware, so `grid_color:(r,g,b,a)` stays one pair
/// rather than splitting on the commas inside it — which is equally what lets
/// a whole persist section (`render:(...)`, `dock:(...)`) be dropped or
/// counted as one.
fn top_level_pairs(blob: &str) -> Vec<(String, String)> {
    let inner = blob.trim().trim_start_matches('(').trim_end_matches(')');
    let (mut out, mut depth, mut start) = (Vec::new(), 0i32, 0usize);
    for (i, c) in inner.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push(inner[start..i].to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(inner[start..].to_string());
    out.into_iter()
        .map(|text| (text[..text.find(':').expect("a pair has a colon")].to_string(), text))
        .collect()
}
