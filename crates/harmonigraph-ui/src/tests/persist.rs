//! What survives a session: round-trips through [`UiPersist`], and the
//! migrations that keep blobs written by older builds loadable.

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
    assert_eq!(restored.view.core_radius, 0.0, "off (radius 0) round-trips");
    assert_eq!(restored.view.core_solidity, 0.4);
    assert_eq!(restored.view.outer_inner, 0.1);
    assert_eq!(restored.view.outer_outer, 0.7);
    assert_eq!(restored.view.idle_marker, harmonigraph_scene::IdleMarker::Dot);
    assert_eq!(restored.view.idle_radius, 0.31);
    assert!(restored.view.mark_melody);
    assert!(!restored.view.mark_bass, "bass off round-trips");
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
    state.view.node_style = harmonigraph_scene::NodeStyle::Vortex;
    let saved = state.save_persist().replace("node_style:Vortex", "node_style:Wire");
    assert_ne!(saved, state.save_persist(), "replacement must have hit");

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&saved);
    assert_eq!(restored.view.node_style, harmonigraph_scene::NodeStyle::Steady);
    assert_eq!(restored.camera.yaw, 1.23, "rest of the blob still restores");
}

#[test]
fn an_old_blobs_octave_style_key_is_ignored_rather_than_fatal() {
    // The octave layer had a style setting (Off, and the Dots/Rings/Petals/
    // Flares/Bumps glyph shapes trimmed before it) until the layer became
    // unconditional. Every one of those tokens still sits in saved blobs, and
    // an unknown key must be SKIPPED — a failed parse drops the whole persist,
    // camera and layout with it, which is a far worse trade than losing a
    // setting that no longer exists.
    for removed in ["Off", "Slices", "Dots", "Rings", "Petals", "Flares", "Bumps"] {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        state.camera.yaw = 1.23;
        state.view.outer_inner = 0.25;
        let saved = state
            .save_persist()
            .replace("outer_inner:", &format!("outer_style:{removed},outer_inner:"));
        assert_ne!(saved, state.save_persist(), "injection must have hit for {removed}");

        let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
        restored.load_persist(&saved);
        assert_eq!(restored.camera.yaw, 1.23, "{removed} took the persist down");
        assert_eq!(restored.view.outer_inner, 0.25, "{removed}");
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
        state.render_config.frame.lattice = LatticeSide::Right;
        state.render_config.frame.split = 0.42;
        let saved = state.save_persist().replace("lattice:Right", &format!("stacked:{flag}"));
        assert_ne!(saved, state.save_persist(), "replacement must have hit for {flag}");

        let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
        restored.load_persist(&saved);
        assert_eq!(restored.render_config.frame.lattice, side, "stacked:{flag}");
        assert_eq!(restored.render_config.frame.split, 0.42, "the rest of the frame survives");
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
    state.render_config.frame.lattice = LatticeSide::Bottom;
    let saved = state.save_persist();
    assert!(saved.contains("lattice:Bottom"), "the side is what gets written");
    assert!(!saved.contains("stacked:"), "the shim must never be written back");

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&saved);
    assert_eq!(restored.render_config.frame.lattice, LatticeSide::Bottom);
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
    assert_eq!(restored.render_config.short_edge, 1440, "the size became the Resolution");
    assert_eq!(restored.render_config.legacy_extra_args, "", "and the text was consumed");
}

#[test]
fn pre_rename_octave_style_and_slice_band_fields_still_load() {
    // The outer layer's band fields were renamed (slice_inner/outer ->
    // outer_inner/outer); aliases must keep blobs with the old names
    // loading, alongside the octave_style key that era also wrote.
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.view.outer_inner = 0.25;
    state.view.outer_outer = 0.85;
    let saved = state
        .save_persist()
        .replace("outer_inner:", "octave_style:Slices,slice_inner:")
        .replace("outer_outer:", "slice_outer:");
    assert_ne!(saved, state.save_persist(), "replacements must have hit");

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&saved);
    assert_eq!(restored.view.outer_inner, 0.25);
    assert_eq!(restored.view.outer_outer, 0.85);
}

#[test]
fn pre_reorg_layout_keeps_its_settings_and_refreshes_the_dock() {
    // The settings tabs were renamed and split (View -> Frame, Appearance ->
    // Nodes + Scene, Spectrum -> Analyzer, Render -> Video, plus a new Panel),
    // Frame was later merged back into Tuning, and the persist blob gained a
    // version. An old blob names the old tabs and has no version. Two things
    // must hold: the `Tab` aliases keep it PARSING (a failed parse silently
    // drops camera/view/spectrum with it), and the absent version refreshes
    // the stale dock so no tab is stranded off-layout or listed twice.
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.camera.yaw = 1.23;
    state.view.extent_sevens = 3;
    // Rewrite a current blob into a pre-reorg one: drop the version field
    // (serde reads it back as 0) and rename the tabs to their old spellings,
    // which only the aliases can still resolve. The capitalized tab tokens
    // don't occur elsewhere in the blob, so these replacements are surgical.
    // The version is spelled from the constant so the next bump doesn't
    // quietly turn the strip into a no-op and leave this testing nothing.
    let saved = state
        .save_persist()
        .replacen(&format!("version:{UI_PERSIST_VERSION},"), "", 1)
        // Tuning is where the old View/Frame tab ended up, so its old name is
        // the one that exercises those aliases.
        .replace("Tuning", "View")
        .replace("Nodes", "Appearance")
        .replace("Analyzer", "Spectrum")
        .replace("Video", "Render");
    assert!(!saved.contains("version:"), "the version strip missed");
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

/// A version-1 layout lists Tuning AND Frame, and the merge made both spell
/// the same variant. Loaded as-is that dock opens with the merged pane in it
/// twice — two tabs, same name, same contents — so the version bump has to
/// refresh it. The settings in the blob must still survive that.
#[test]
fn a_pre_merge_layout_does_not_open_the_merged_tab_twice() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.camera.yaw = 0.77;
    state.spectrum_config.floor_db = -42.0;
    // Synthesize the version-1 blob: same layout, but with the Frame tab still
    // sitting next to Tuning where it used to be.
    let saved = state
        .save_persist()
        .replacen(&format!("version:{UI_PERSIST_VERSION},"), "version:1,", 1)
        .replacen("Tuning,", "Tuning,Frame,", 1);
    assert!(saved.contains("Frame"), "the synthetic v1 layout must name Frame");

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&saved);
    // Parsed: `Frame` still resolves (onto Tuning), so nothing was dropped.
    assert_eq!(restored.camera.yaw, 0.77, "settings survive the pre-merge layout");
    assert_eq!(restored.spectrum_config.floor_db, -42.0);
    // And the duplicate is gone rather than carried into the dock.
    let dock = ron::to_string(&restored.dock).unwrap();
    assert_eq!(
        dock,
        ron::to_string(&default_dock()).unwrap(),
        "a pre-merge layout resets to the current default dock",
    );
    assert_eq!(dock.matches("Tuning").count(), 1, "the merged tab is docked twice");
}

/// A refreshed dock has to take the folds with it.
///
/// `Folds` remembers a split by INDEX — surface and node into the dock tree —
/// plus the fraction to give back on unfold. Those indices mean nothing once
/// the tree is replaced, which is why "Reset layout" calls `Folds::clear`. The
/// version bump in `load_persist` replaces the tree just as wholesale, and its
/// own comment says so ("the remembered fractions would name splits in a tree
/// that is gone") — but it sits on the branch that KEEPS the dock, so the
/// branch that throws it away never cleared anything.
///
/// The shared UI state outlives the editor window (`editor.rs`: "This is a NEW
/// context; the shared UI state is not"), so a fold recorded while the window
/// was open is still in memory when an older blob arrives on the same device —
/// load a pre-merge project or preset onto it, and the default layout comes
/// back with one split sitting at a fraction measured against a tree that no
/// longer exists.
#[test]
fn a_refreshed_dock_forgets_the_folds_measured_against_the_old_one() {
    let state = SharedState::new(TextureFormat::Bgra8Unorm);
    let saved = state.save_persist();
    // A fold on the node the default dock does NOT have folded, so a fraction
    // left behind is visible as a layout that is not the default.
    let folded = saved.replacen("folds:([])", "folds:([(surface:0,node:1,fraction:0.9)])", 1);
    assert_ne!(folded, saved, "the folds field must have been there to splice onto");

    // It loads at the current version, which is how it gets into memory at all.
    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&folded);
    assert!(!restored.folds.is_empty(), "the spliced fold must have loaded");

    // Now the same instance is handed an older blob: the dock is refreshed to
    // the current default, so what the folds named is gone.
    let old = folded.replacen(&format!("version:{UI_PERSIST_VERSION},"), "version:1,", 1);
    assert_ne!(old, folded, "the version rewrite missed");
    restored.load_persist(&old);
    assert_eq!(
        ron::to_string(&restored.dock).unwrap(),
        ron::to_string(&default_dock()).unwrap(),
        "the premise: an old version refreshes the dock",
    );
    assert!(
        restored.folds.is_empty(),
        "folds kept indices into a dock that was just thrown away",
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
fn node_body_experiment_blobs_fold_into_the_core() {
    // Blobs saved by the one-build NodeBody experiment carry a
    // node_body field the current layout no longer writes; loading one
    // must both parse and fold the body into the core/outer split
    // (Beads = the core glow, solidity 0, the octave layer carrying the
    // note). They wrote the legacy core_style:Orb.
    let state = SharedState::new(TextureFormat::Bgra8Unorm);
    let saved = state
        .save_persist()
        .replace("core_solidity:", "core_style:Orb,node_body:Beads,core_solidity:");
    assert_ne!(saved, state.save_persist(), "injection must have hit");

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&saved);
    assert_eq!(restored.view.core_solidity, 0.0, "octave-only body is the glow end");
    assert!(restored.view.core_radius > 0.0, "still on");
    assert_eq!(
        restored.view.node_body,
        harmonigraph_scene::LegacyNodeBody::Disc,
        "shim consumed on load"
    );
}

#[test]
fn retired_octave_backdrop_and_solidity_keys_do_not_sink_a_blob() {
    // The backdrop and the octave glyphs' solidity were settings before
    // both were fixed at 1, and saved blobs still carry the keys they rode
    // on — the backdrop as a bare bool, then as an opacity under
    // `outer_backdrop_alpha`. Their fields are gone, so serde skips them as
    // unknown; what must not happen is a parse error, because load_persist
    // drops the WHOLE blob on one and the user would silently lose their
    // layout, camera and every other view setting along with it.
    for keys in [
        "outer_backdrop:true,",
        "outer_backdrop:false,",
        "outer_backdrop_alpha:0.5,",
        "outer_solidity:0.3,",
        "outer_backdrop_alpha:0.5,outer_solidity:0.3,",
    ] {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        // Something non-default elsewhere in the blob, so surviving the
        // load means more than landing back on the defaults.
        state.view.extent_threes = 7;
        let saved = state
            .save_persist()
            .replace("core_solidity:", &format!("{keys}core_solidity:"));
        assert_ne!(saved, state.save_persist(), "injection must have hit for {keys}");

        let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
        restored.load_persist(&saved);
        assert_eq!(restored.view.extent_threes, 7, "blob survived {keys}");
    }
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

/// The pitch range used to be a pair of Bitwig octave numbers. A blob from
/// then carries `low_octave`/`high_octave` and no `low_midi`, so without the
/// fold serde hands it the full-axis default and the zoom the user set is
/// silently gone.
#[test]
fn an_octave_numbered_pitch_range_migrates_to_midi() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.spectrum_config.low_midi = 40.5;
    state.spectrum_config.high_midi = 100.0;
    // C1..C5 in Bitwig numbering — MIDI 36..84.
    let old = state
        .save_persist()
        .replace("low_midi:40.5", "low_octave:1")
        .replace("high_midi:100.0", "high_octave:5");
    assert!(old.contains("low_octave:1"), "the replacement must have hit");

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&old);
    assert_eq!(restored.spectrum_config.low_midi, 36.0);
    assert_eq!(restored.spectrum_config.high_midi, 84.0);
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
