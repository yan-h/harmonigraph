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
    // Radius 0 is the off state; this proves it (and solidity) persist.
    state.view.core_radius = 0.0;
    state.view.core_solidity = 0.4;
    state.view.outer_inner = 0.1;
    state.view.outer_outer = 0.7;
    state.view.outer_backdrop = 0.62;
    state.view.outer_solidity = 0.3;
    state.view.idle_marker = lattice_scene::IdleMarker::Dot;
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
    assert_eq!(restored.view.outer_backdrop, 0.62);
    assert_eq!(restored.view.outer_solidity, 0.3);
    assert_eq!(restored.view.idle_marker, lattice_scene::IdleMarker::Dot);
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
    state.view.node_style = lattice_scene::NodeStyle::Vortex;
    let saved = state.save_persist().replace("node_style:Vortex", "node_style:Wire");
    assert_ne!(saved, state.save_persist(), "replacement must have hit");

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&saved);
    assert_eq!(restored.view.node_style, lattice_scene::NodeStyle::Steady);
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
fn node_body_experiment_blobs_fold_into_the_core_and_backdrop() {
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

/// Every tab needs its own id, and the titles cannot supply one: the display
/// pane and its settings are both called "Analyzer" on purpose. egui_dock's
/// default `id()` is the title text, and that id keys the tab BODY's `Ui`
/// (surface + tab id, no node), so a collision made two panes share their
/// body state — scrolling the settings scrolled the analyzer display.
#[test]
fn every_tab_has_its_own_id_even_where_two_share_a_title() {
    use egui_dock::TabViewer;
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let params = RecordingBackend::default();
    let tabs = [
        panes::Tab::Lattice,
        panes::Tab::Tuning,
        panes::Tab::Nodes,
        panes::Tab::Scene,
        panes::Tab::Console,
        panes::Tab::Spectral,
        panes::Tab::Analyzer,
        panes::Tab::Notes,
        panes::Tab::Video,
        panes::Tab::Panel,
    ];
    let mut viewer = panes::Viewer { state: &mut state, params: &params, now: 0.0 };
    let mut title = |mut tab: panes::Tab| viewer.title(&mut tab).text().to_owned();

    // The collision this guards against is real, not hypothetical.
    assert_eq!(
        title(panes::Tab::Spectral),
        title(panes::Tab::Analyzer),
        "the two Analyzer tabs are meant to share a title",
    );

    let ids: Vec<egui::Id> = tabs
        .iter()
        .map(|&tab| {
            let mut tab = tab;
            viewer.id(&mut tab)
        })
        .collect();
    for (i, a) in ids.iter().enumerate() {
        for (j, b) in ids.iter().enumerate().skip(i + 1) {
            assert_ne!(a, b, "{:?} and {:?} share a tab id", tabs[i], tabs[j]);
        }
    }
}

/// The picture panes fill their body exactly, so a scroll area around one can
/// only shift a picture that is meant to sit still.
#[test]
fn the_picture_panes_do_not_scroll() {
    use egui_dock::TabViewer;
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let params = RecordingBackend::default();
    let viewer = panes::Viewer { state: &mut state, params: &params, now: 0.0 };
    for tab in [panes::Tab::Lattice, panes::Tab::Spectral] {
        assert_eq!(viewer.scroll_bars(&tab), [false, false], "{tab:?} is scrollable");
    }
    // Settings panes are lists and must stay reachable in a short column, but
    // VERTICALLY only: a both-axes area gives the body unbounded width, and the
    // panes that fill the space then never report vertical overflow, so the
    // wheel can't scroll them. Horizontal off; vertical on.
    for tab in [panes::Tab::Tuning, panes::Tab::Analyzer, panes::Tab::Panel] {
        assert_eq!(viewer.scroll_bars(&tab), [false, true], "{tab:?} cannot scroll vertically");
    }
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
/// on that too — a handler that only reads the scroll delta does nothing here.
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
    assert!(spectrum.display(0.0).is_none(), "no audio yet");

    // A 440 Hz sine, long enough to fill the analysis window.
    let sine: Vec<f32> = (0..9_000)
        .map(|i| 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin())
        .collect();
    spectrum.push_samples(&sine, 1, 48_000.0, 1.0, &config);
    let levels = spectrum.display(1.0).expect("audio is flowing");
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
    assert!(spectrum.display(1.0 + AudioSpectrum::HOLD_SECONDS + 0.1).is_none());
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
    state.spectrum_config.spectrogram_opacity = 0.5;
    state.spectrum_config.spectrogram_gamma = 1.6;
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
    assert_eq!(restored.spectrum_config.spectrogram_opacity, 0.5);
    assert_eq!(restored.spectrum_config.spectrogram_gamma, 1.6);
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
    use lattice_core::spectrum::{SPECTRUM_MAX_MIDI, SPECTRUM_MIN_MIDI};
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
/// it. `spectrogram_fine_levels` existed only while the heatmap's stored
/// precision was being judged by eye, so any project saved during that window
/// carries it — and a blob that fails to parse loses the entire UI state, not
/// just the stale key.
#[test]
fn a_persist_blob_carrying_a_since_removed_field_still_loads() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.view.extent_sevens = 3;
    let saved = state.save_persist();
    // Put the departed field back, exactly as that build wrote it.
    let stale = saved
        .replace("spectrogram_gamma:", "spectrogram_fine_levels:true,spectrogram_gamma:");
    assert_ne!(stale, saved, "the anchor field must have been there to splice onto");

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&stale);
    assert_eq!(restored.view.extent_sevens, 3, "an unknown field must not sink the blob");
}

#[test]
fn spectrogram_history_stays_bounded() {
    let bins = [0.0f32; lattice_core::spectrum::SPECTRUM_BINS];

    // Retention is span-INDEPENDENT: it must NOT track the current window, or
    // shrinking the span pops history that widening it again can never recover
    // (the "reducing then increasing span erases history" bug). Everything
    // within HISTORY_MAX_SECONDS is kept no matter what the span is doing.
    let mut spec = AudioSpectrum::default();
    for i in 0..300 {
        spec.push_history(i as f64, &bins);
    }
    assert_eq!(spec.history().front().unwrap().time, 0.0, "no column within the cap is dropped");
    assert_eq!(spec.history().back().unwrap().time, 299.0, "newest kept");

    // The retention never exceeds the hard age cap — the ceiling on how far
    // back the heatmap can read.
    let mut spec = AudioSpectrum::default();
    for i in 0..800 {
        spec.push_history(i as f64, &bins);
    }
    let cutoff = 799.0 - AudioSpectrum::HISTORY_MAX_SECONDS;
    assert!(spec.history().front().unwrap().time >= cutoff, "capped at HISTORY_MAX_SECONDS");

    // Memory holds even when every column shares one timestamp, so the age trim
    // never fires — the store's own tier caps are the backstop.
    let mut spec = AudioSpectrum::default();
    for _ in 0..(SpectrumHistory::MAX_COLUMNS + 50) {
        spec.push_history(0.0, &bins);
    }
    assert!(spec.history().len() <= SpectrumHistory::MAX_COLUMNS, "column count capped");

    spec.clear_history();
    assert!(spec.history().is_empty());
}

/// The live path stamps its columns the same way the offline one does, and
/// the near-edge grace knows about the lag that creates. Two halves of one
/// thing: a column half a window old is not a stale column, it is the newest
/// there can be, and the heatmap must still reach the now-line.
#[test]
fn a_live_column_is_stamped_at_the_middle_of_its_window() {
    let mut spectrum = AudioSpectrum::default();
    let config = SpectrumConfig::default();
    // A WHOLE number of hops, so the last column's window ends exactly at `now`
    // and the stamp can be checked exactly. A spectrum is taken every
    // FFT_INTERVAL of audio (see `push_samples`), so a batch ending mid-hop
    // leaves its newest column up to one hop further back than this — correctly,
    // since that is where the window it measured ends.
    let hop = (AudioSpectrum::FFT_INTERVAL * 48_000.0).round() as usize;
    let samples = hop * (9_000 / hop + 1); // enough to fill the 8192 window
    let sine: Vec<f32> = (0..samples)
        .map(|i| 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin())
        .collect();
    let now = 5.0;
    spectrum.push_samples(&sine, 1, 48_000.0, now, &config);
    spectrum.display(now).expect("audio is flowing");

    let window = f64::from(config.window.samples() as u32) / 48_000.0;
    let stamped = spectrum.history().back().expect("a column was kept").time;
    assert!(
        (stamped - (now - window * 0.5)).abs() < 1e-9,
        "a column measured over [{:.3}, {now}] was stamped {stamped}, not its middle",
        now - window,
    );
    // And the pane's own idea of that lag agrees, which is what keeps the
    // strip's near edge on the now-line rather than half a window short.
    assert!((spectrum.column_lag() - window * 0.5).abs() < 1e-9);
}

/// The column grid is a function of the SAMPLES, not of when the shell happened
/// to hand them over — which is the whole reason the FFT moved into
/// `push_samples`. A shell drains its audio ring on frame boundaries while the
/// ring fills in audio blocks, so batch sizes swing by a block and the frame
/// clock wobbles against the audio clock by several ms; the old frame-gated FFT
/// passed all of that into the picture. It could only fire ON a frame, so a
/// 20 ms interval on a 60 Hz display fired every 33.3 ms — wider than the slabs
/// the heatmap cuts the window into, which then went empty and were painted by
/// duplicating a neighbour.
///
/// Hence the second assertion, which is the one the eye sees: no gap wider than
/// `MIN_BUCKET` means no slab is ever empty, at any frame rate or cap.
#[test]
fn columns_are_evenly_spaced_however_the_shell_batches_them() {
    use crate::panes::spectrogram::MIN_BUCKET;
    let sr = 48_000.0f32;
    let config = SpectrumConfig::default();
    let mut spectrum = AudioSpectrum::default();
    let mut written = 0usize;
    for batch in 0..48 {
        // Sizes a block apart, and a frame clock that leads and lags the audio
        // it is dating by 4 ms — twice what it takes to lose a column.
        let n = [512usize, 256, 1024, 128, 768][batch % 5];
        let chunk: Vec<f32> = (0..n)
            .map(|k| {
                let t = (written + k) as f32 / sr;
                0.5 * (std::f32::consts::TAU * 440.0 * t).sin()
            })
            .collect();
        written += n;
        let now = f64::from(written as u32) / f64::from(sr)
            + if batch % 2 == 0 { 0.004 } else { -0.004 };
        spectrum.push_samples(&chunk, 1, sr, now, &config);
    }

    let times: Vec<f64> = spectrum.history().iter().map(|c| c.time).collect();
    assert!(times.len() > 20, "only {} columns for 48 batches", times.len());
    let hop = AudioSpectrum::FFT_INTERVAL;
    for pair in times.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(
            (gap - hop).abs() < hop * 0.25,
            "columns {:.4} s apart, not {hop} — the batching is reaching the grid",
            gap,
        );
        assert!(gap < MIN_BUCKET, "a {gap:.4} s gap can leave a {MIN_BUCKET} s slab empty");
    }
}

/// The live pane and the offline render must analyze stereo IDENTICALLY, or a
/// video would differ from the look it was dialed in against — and only for
/// stereo-wide material, which is the hardest kind of difference to attribute to
/// its cause. They share `ChannelBank` so that this holds by construction; this
/// is what says the sharing actually reaches both paths.
///
/// The signal is deliberately one a mono mixdown would mangle: an anti-phase A4
/// (erased entirely by a sum) under an in-phase E5. If either path mixed down,
/// its columns would be missing a partial the other one has.
#[test]
fn the_live_path_and_the_offline_precompute_agree_on_stereo() {
    use lattice_core::spectrum::midi_to_hz;
    let sr = 48_000.0f32;
    let frames = 48_000usize; // one second
    let (a4, e5) = (midi_to_hz(69.0), midi_to_hz(76.0));
    let samples: Vec<f32> = (0..frames)
        .flat_map(|i| {
            let t = i as f32 / sr;
            let anti = 0.6 * (std::f32::consts::TAU * a4 * t).sin();
            let both = 0.3 * (std::f32::consts::TAU * e5 * t).sin();
            [both + anti, both - anti]
        })
        .collect();
    let cfg = SpectrumConfig::default();
    let span = f64::from(frames as u32) / f64::from(sr);

    // Live: one batch, dated so the newest frame sits at the end of the second.
    let mut spectrum = AudioSpectrum::default();
    spectrum.push_samples(&samples, 2, sr, span, &cfg);
    let live: Vec<_> = spectrum.history().iter().map(|c| (c.time, c.db.clone())).collect();

    // Offline: the same buffer, the whole-song build.
    let ws = WholeSong::precompute(&samples, 2, sr, 0.0, 0.0, span, &cfg);
    let offline: Vec<_> = ws.columns.iter().map(|c| (c.time, c.db.clone())).collect();

    assert!(live.len() > 50, "only {} live columns for a second of audio", live.len());
    assert_eq!(live.len(), offline.len(), "different column counts");
    for (i, ((lt, ldb), (ot, odb))) in live.iter().zip(&offline).enumerate() {
        assert!((lt - ot).abs() < 1e-6, "column {i} stamped {lt} live, {ot} offline");
        assert!(ldb == odb, "column {i} holds different buckets live and offline");
    }

    // And both really did keep the anti-phase partial — otherwise the two could
    // agree by both being wrong in the same way.
    let bucket_of = |hz: f32| {
        ((lattice_core::spectrum::hz_to_midi(hz) - lattice_core::spectrum::SPECTRUM_MIN_MIDI)
            * lattice_core::spectrum::BINS_PER_SEMITONE as f32)
            .round() as usize
    };
    let loudest = |bucket: usize| {
        live.iter()
            .flat_map(|(_, db)| db[bucket.saturating_sub(1)..=bucket + 1].iter().copied())
            .max()
            .unwrap_or(0)
    };
    assert!(
        loudest(bucket_of(a4)) > loudest(bucket_of(e5)) / 2,
        "the anti-phase A4 is missing: {} against E5's {}",
        loudest(bucket_of(a4)),
        loudest(bucket_of(e5)),
    );
}

/// A spectrum is measured over a WINDOW, not at an instant, so where it lands
/// on the time axis is a choice — and the only defensible one is the middle of
/// what it measured. Stamping it when the FFT ran (the end of that window) drew
/// every sound half a window late: at the default 8192 that is 85 ms, so a note
/// ribbon sat 85 ms further from the now-line than the energy it made, and
/// reached the far edge — and vanished — that much before its own audio did.
///
/// Checked where it is measurable rather than argued: a tone that starts at a
/// known moment must light the heatmap at that moment.
#[test]
fn a_tones_energy_lands_at_the_time_the_tone_started() {
    use lattice_core::spectrum::midi_to_hz;
    let sr = 48_000.0f32;
    let onset = 1.0f64; // silence before this, a steady A4 after
    let seconds = 3.0;
    let freq = midi_to_hz(69.0);
    let samples: Vec<f32> = (0..(sr as f64 * seconds) as usize)
        .map(|i| {
            let t = f64::from(i as u32) / f64::from(sr);
            if t < onset {
                0.0
            } else {
                0.8 * (std::f32::consts::TAU * freq * i as f32 / sr).sin()
            }
        })
        .collect();
    let cfg = SpectrumConfig::default();
    let ws = WholeSong::precompute(&samples, 1, sr, 0.0, 0.0, seconds, &cfg);

    // The bin the tone sits in, and how loud it reads once fully sounding.
    // Columns are stored as bytes of dB, so "half power" is 3 dB down from the
    // peak rather than half its stored value.
    use lattice_core::spectrogram::{db_of, DB_STEP};
    let a4 = ((69.0 - lattice_core::spectrum::SPECTRUM_MIN_MIDI)
        * lattice_core::spectrum::BINS_PER_SEMITONE as f32)
        .round() as usize;
    let loudest = ws.columns.iter().map(|c| c.db[a4]).max().expect("columns");
    assert!(db_of(loudest) > -10.0, "the tone should read loudly at its own bin");

    // Where the ridge reaches half power is what the eye reads as the onset:
    // the window is Hann-weighted, so it crosses half when it is half over the
    // start of the tone. That must be the moment the tone started.
    let half_power = loudest.saturating_sub((3.01 / DB_STEP).round() as u8);
    let half = ws
        .columns
        .iter()
        .find(|c| c.db[a4] >= half_power)
        .expect("the tone must reach half power somewhere");
    let window = f64::from(cfg.window.samples() as u32) / f64::from(sr);
    assert!(
        (half.time - onset).abs() < window * 0.25,
        "the tone starts at {onset} s but its energy reads as starting at {} s \
         (a {window:.3} s window; half of one late would be the old end-stamping)",
        half.time,
    );
}

/// The store has to be sized for the retention policy above it: every second
/// inside `HISTORY_MAX_SECONDS` must have columns to draw, or a long span shows
/// a heatmap that stops partway and bare roll beyond it (which is exactly what
/// a fixed-rate ring does — 160 MB buys 3.5 minutes of a 10 minute span).
/// Raising the cap means adding a tier; this is what says so.
#[test]
fn spectrum_history_reaches_the_retention_cap() {
    let reach = SpectrumHistory::reach(AudioSpectrum::FFT_INTERVAL);
    assert!(
        reach >= AudioSpectrum::HISTORY_MAX_SECONDS,
        "history reaches {reach:.0} s, retention asks for {:.0} s — add a tier",
        AudioSpectrum::HISTORY_MAX_SECONDS,
    );
    // And it fits in a budget worth calling an optimization: the fixed-rate
    // f32 ring needed 160 MB to reach a third as far.
    //
    // 15 -> 30 MB was bought deliberately, and by the DISPLAY rather than by
    // reach: `LIVE_SLAB_CAP` doubled to 1024 so the default 12 s span is cut
    // into slabs as fine as the data, and the tiers have to keep up with the cap
    // (see COARSE_COLUMNS) — so the cap, the tier size, and this number are one
    // decision. Reach came along for free.
    let megabytes = SpectrumHistory::max_bytes() as f64 / (1024.0 * 1024.0);
    assert!(megabytes < 32.0, "the full store is {megabytes:.1} MB");
}

/// The bargain the tiers are struck on: a column of age `a` is only ever drawn
/// when the window is at least `a` long, and a window is cut into at most
/// `LIVE_SLAB_CAP` slabs — so nothing needs storing finer than `a / cap`. Every
/// tier must stay on the right side of that, or its columns land more than a
/// slab apart and the heatmap grows stripes of false silence between them.
///
/// This is the test to look at if the tier sizes, the FFT rate, or the slab cap
/// ever move: they are three legs of one stool.
///
/// The display now picks its slab off the same power-of-two ladder the tiers
/// merge on (`live_slab`), so the two round to the same rung rather than merely
/// bounding each other — but the bargain is what makes that ladder the right
/// one, so it is still worth stating against the real function.
#[test]
fn stored_columns_stay_finer_than_the_slabs_they_are_drawn_into() {
    use crate::panes::spectrogram::{live_slab, LIVE_SLAB_CAP};
    let mut age = 0.0f64; // youngest age the tier holds
    let mut spacing = AudioSpectrum::FFT_INTERVAL;
    for tier in 0..SpectrumHistory::TIERS {
        // The finest slab any window that reaches this tier's youngest columns
        // can use — the tightest the tier is ever asked to be.
        let finest = live_slab(age, LIVE_SLAB_CAP as usize);
        assert!(
            spacing <= finest,
            "tier {tier} stores {spacing:.3} s apart but can be drawn into \
             {finest:.3} s slabs (from age {age:.1} s)",
        );
        let columns =
            if tier == 0 { SpectrumHistory::FINE_COLUMNS } else { SpectrumHistory::COARSE_COLUMNS };
        age += columns as f64 * spacing;
        spacing *= 2.0;
    }
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

    let ws = WholeSong::precompute(&samples, 1, sr, 0.0, 0.0, seconds, &cfg);
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
    let peak = (0..SPECTRUM_BINS).max_by_key(|&b| mid.db[b]).unwrap();
    assert!(peak.abs_diff(a4) <= 1, "peak bin {peak} should be A4 (bin {a4})");

    // `time_origin` shifts every column onto the take's timeline.
    let shifted = WholeSong::precompute(&samples, 1, sr, 5.0, 0.0, seconds, &cfg);
    assert!(
        (shifted.columns[0].time - ws.columns[0].time - 5.0).abs() < 1e-6,
        "time_origin offsets the columns"
    );

    // Pure: same inputs in, byte-identical columns out (the render leans on
    // this for reproducibility).
    let again = WholeSong::precompute(&samples, 1, sr, 0.0, 0.0, seconds, &cfg);
    assert_eq!(ws.columns.len(), again.columns.len());
    for (a, b) in ws.columns.iter().zip(&again.columns) {
        assert_eq!(a.time, b.time);
        assert_eq!(a.db, b.db, "precompute is deterministic");
    }
}

/// The box one piece of text occupies.
fn text_box(texts: &[(egui::Rect, String)], want: &str) -> egui::Rect {
    texts
        .iter()
        .filter(|(_, t)| t == want)
        .map(|(r, _)| *r)
        .reduce(|a, b| a.union(b))
        .unwrap_or_else(|| panic!("no {want:?} drawn, got {texts:?}"))
}

/// The stroke weight the label tests draw their marks at. Nothing below
/// depends on the number; the marks only have to be present and placed.
const TEST_MARK_WEIGHT: f32 = 0.10;

/// A label's text pieces AND the boxes of its drawn marks.
///
/// The comma signs are geometry rather than type (see
/// [`panes::lattice::draw_stacked_name`]), so a text-only view of a label is
/// blind to exactly the marks these tests are about. Each drawn piece emits
/// two shapes, halo then fill, and both are symmetric about the piece, so a
/// box here is the piece's own box grown by the halo.
fn drawn_label(
    name: lattice_core::NoteName,
    anchor: egui::Pos2,
) -> (Vec<(egui::Rect, String)>, Vec<egui::Rect>) {
    let ctx = egui::Context::default();
    theme::apply_theme(&ctx); // the real Iosevka metrics, not egui's fallback
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
    let mut batch = crate::text::TextBatch::default();
    let out = ctx.run_ui(
        egui::RawInput { screen_rect: Some(screen), ..Default::default() },
        |ui| {
            panes::lattice::draw_stacked_name(
                &mut batch,
                ui.painter(),
                anchor,
                name,
                egui::Color32::WHITE,
                egui::Color32::BLACK,
                1.0,
                TEST_MARK_WEIGHT,
            );
        },
    );
    let texts = batch.pieces().iter().map(|p| (p.galley, p.text.clone())).collect();
    let shapes = out
        .shapes
        .iter()
        .map(|s| s.shape.visual_bounding_rect())
        .filter(|r| r.is_finite() && r.width() > 0.0 && r.height() > 0.0)
        .collect();
    (texts, shapes)
}

/// The lattice's note labels stack the accidental over the comma sign in one
/// column after the letter, so a name deep in the lattice stays narrow. The
/// whole name still has to sit centered on its node.
#[test]
fn note_label_stacks_the_marks_and_stays_centered_on_the_node() {
    let anchor = egui::pos2(200.0, 200.0);
    let name = lattice_core::NoteName {
        letter: 'C',
        sharps: 5,
        syntonic_commas: 4,
        septimal_commas: 0,
    };
    let (texts, shapes) = drawn_label(name, anchor);

    // Counted marks, not five sharps and four pluses spelled out. The `+`
    // itself is drawn, so only its COUNT is text.
    let letter = text_box(&texts, "C");
    let accidental = text_box(&texts, "\u{266F}5");
    let count = text_box(&texts, "4");
    let sign = shapes
        .iter()
        .copied()
        .reduce(|a, b| a.union(b))
        .expect("the + should be drawn, not typeset");

    // One column, beginning where the letter ends. The boxes are ink, so
    // they meet within a glyph's own side bearing rather than within the
    // rim a stamped box would carry.
    const BEARING: f32 = 2.0;
    assert!(
        (accidental.left() - letter.right()).abs() <= 2.0 * BEARING,
        "marks should follow the letter ({accidental:?} after {letter:?})"
    );
    assert!(
        (accidental.left() - sign.left()).abs() <= 2.0 * BEARING,
        "the drawn sign shares the accidental's column ({sign:?} vs {accidental:?})"
    );
    assert!(sign.right() <= count.left() + BEARING, "the count follows its sign");
    // Superscript over subscript, straddling the letter's own line.
    assert!(accidental.center().y < letter.center().y, "the accidental rides high");
    assert!(sign.center().y > letter.center().y, "the comma sits low");
    // Marks are subordinate to the letter, not the same weight...
    assert!(accidental.height() < letter.height(), "marks are the smaller size");
    // ...and neither stands proud of it: the stacked pair has to stay inside
    // the letter's own height, or the label reads as two lines, not one name.
    assert!(
        accidental.top() >= letter.top() - 0.01 && count.bottom() <= letter.bottom() + 0.01,
        "marks should not overhang the letter (acc {accidental:?}, count {count:?}, \
         letter {letter:?})"
    );

    // The name as a whole straddles the node it labels.
    let name_box = letter.union(accidental).union(count);
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

/// The septimal mark sits ACROSS the divide between the accidental and the
/// comma, not in either slot.
///
/// It belongs to a different prime than the two it sits beside, and it is
/// placed to say so: centered on the letter's own line, with air before it.
/// It used to take one slot or the other as a second cue for its direction,
/// which put it in the stack it is not part of; the chevron carries its own
/// direction, so the slot is free to mean this instead.
#[test]
fn the_septimal_mark_sits_across_the_divide_between_the_other_two() {
    let anchor = egui::pos2(200.0, 200.0);
    let mark_of = |septimal_commas: i32| {
        let name = lattice_core::NoteName {
            letter: 'B',
            sharps: -1,
            syntonic_commas: 0,
            septimal_commas,
        };
        let (_, shapes) = drawn_label(name, anchor);
        shapes.into_iter().reduce(|a, b| a.union(b)).expect("a septimal mark should be drawn")
    };
    // Both directions sit on the same line, and it is the letter's own.
    for commas in [-1, 1] {
        let mark = mark_of(commas);
        assert!(
            (mark.center().y - anchor.y).abs() < 1.0,
            "a septimal mark belongs on the letter's line, got {mark:?} against {anchor:?}"
        );
    }
    // A home-sheet name draws no mark at all.
    let (_, none) = drawn_label(
        lattice_core::NoteName {
            letter: 'B',
            sharps: -1,
            syntonic_commas: 0,
            septimal_commas: 0,
        },
        anchor,
    );
    assert!(none.is_empty(), "no sevens component, no mark: {none:?}");
}

/// A mark costs a bounded few quads, exactly as the glyphs beside it do.
///
/// This is the invariant `crate::text` exists to hold: a label's rim is
/// stamped in the FRAGMENT stage precisely because "20 copies of every glyph
/// was most of the geometry in a busy frame". `paint_mark` reasoned its way
/// out of that with "a mark is one quad, so the loop is affordable here" --
/// true of the handful of hovered and sounding nodes it was written against,
/// and false the moment note names put a mark on every roll ribbon and every
/// lit node of a collapsed 12-EDO lattice.
///
/// A count, not a timing, so it cannot go quiet on a fast machine.
#[test]
fn a_mark_costs_a_bounded_number_of_quads() {
    let anchor = egui::pos2(200.0, 200.0);
    let name = lattice_core::NoteName {
        letter: 'E',
        sharps: 0,
        syntonic_commas: -1,
        septimal_commas: -1,
    };
    let (_, shapes) = drawn_label(name, anchor);
    assert!(
        shapes.len() <= 4,
        "two marks cost {} quads; the rim belongs in the fragment stage, \
         not once per stamp in the shape list",
        shapes.len()
    );
}

/// The septimal mark gets a column of its own, so a name carrying both
/// commas reads as three pieces rather than a pile.
#[test]
fn both_comma_marks_get_their_own_column() {
    let anchor = egui::pos2(200.0, 200.0);
    let name = lattice_core::NoteName {
        letter: 'E',
        sharps: 0,
        syntonic_commas: -1,
        septimal_commas: -1,
    };
    let (texts, shapes) = drawn_label(name, anchor);
    assert!(texts.iter().all(|(_, t)| t == "E"), "single marks carry no count: {texts:?}");
    let letter = text_box(&texts, "E");
    // Two drawn marks, so two columns' worth of shapes: the syntonic bar
    // sits left of the septimal shape rather than on top of it.
    let left = shapes.iter().copied().reduce(|a, b| a.union(b)).expect("marks drawn");
    assert!(left.left() >= letter.right() - 2.0, "marks follow the letter, {left:?}");

    // Cluster the stamps into columns rather than reading the flat list's
    // extremes. A mark is not one shape: `paint_mark` stamps its rim as ~20
    // separate quads around the fill, so ONE mark already spreads its centers
    // over four points, and `min(x) < max(x)` holds with the other mark
    // missing entirely or drawn on top of it -- the two failures this test
    // exists to catch. Within a mark consecutive stamps are under a point
    // apart; between the columns they are nearly two.
    const COLUMN_GAP: f32 = 1.0;
    let mut stamps: Vec<egui::Pos2> = shapes.iter().map(|r| r.center()).collect();
    stamps.sort_by(|a, b| a.x.total_cmp(&b.x));
    let mut columns: Vec<Vec<egui::Pos2>> = Vec::new();
    for stamp in stamps {
        match columns.last_mut() {
            Some(col) if stamp.x - col[col.len() - 1].x <= COLUMN_GAP => col.push(stamp),
            _ => columns.push(vec![stamp]),
        }
    }
    let widths: Vec<usize> = columns.iter().map(|c| c.len()).collect();
    assert_eq!(columns.len(), 2, "two marks, two columns, got {widths:?}");
    assert_eq!(widths[0], widths[1], "each column is a whole mark, not one mark's rim");

    // The right-hand column is the septimal one, and it is on the letter's own
    // line while the syntonic bar sits below it -- so which column is which is
    // checked, not assumed from the ordering the split already imposed.
    let line = |col: &[egui::Pos2]| col.iter().map(|p| p.y).sum::<f32>() / col.len() as f32;
    assert!(
        line(&columns[1]) < line(&columns[0]),
        "the septimal mark takes the right column, across the letter's line: {:?}",
        columns.iter().map(|c| line(c)).collect::<Vec<_>>()
    );
}

/// A plain name has no marks to stack -- nothing extra is drawn, and the
/// letter alone centers on the node.
#[test]
fn a_natural_note_label_is_just_the_letter() {
    let anchor = egui::pos2(200.0, 200.0);
    let name = lattice_core::NoteName {
        letter: 'G',
        sharps: 0,
        syntonic_commas: 0,
        septimal_commas: 0,
    };
    let (texts, shapes) = drawn_label(name, anchor);
    assert!(texts.iter().all(|(_, t)| t == "G"), "only the letter: {texts:?}");
    assert!(shapes.is_empty(), "a natural draws no marks: {shapes:?}");
    assert!((text_box(&texts, "G").center().x - anchor.x).abs() < 0.5);
}


/// The cents readout hangs off the note name's GLYPHS, not its galley box --
/// a monospace line box carries several pixels of leading below the letter,
/// and spacing box-to-box left the readout visibly adrift from the name it
/// belongs to.
#[test]
fn the_cents_readout_sits_right_under_the_note_name() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.view.show_labels = true;
    state.view.show_cents = true;
    // Middle C: the origin node, which the default camera looks straight at.
    state.tracker.handle_event(lattice_core::NoteEvent {
        time: 0.0,
        channel: 0,
        note: 60,
        kind: lattice_core::NoteEventKind::On { velocity: 1.0 },
    });
    let scene = lattice_scene::derive_scene(
        &state.tracker,
        &state.tuning,
        &state.view,
        &state.frame_params,
        state.camera,
        None,
        0.0,
    );

    let ctx = egui::Context::default();
    theme::apply_theme(&ctx); // the real Iosevka metrics, not egui's fallback
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 800.0));
    let mut batch = crate::text::TextBatch::default();
    let _ = ctx.run_ui(
        egui::RawInput { screen_rect: Some(rect), time: Some(0.0), ..Default::default() },
        |ui| panes::lattice::draw_node_labels(ui, rect, &scene, &state.view, 1.0, &mut batch),
    );

    // A held note lights every node of its pitch class, so each piece turns
    // up once per lit node. Sort them by the label's own type sizes, which
    // nothing else in the pane shares -- not by the text, since one pitch
    // class is spelled several ways across the lattice.
    let ink_of = |want: &[f32]| -> Vec<egui::Rect> {
        let mut clusters: Vec<egui::Rect> = Vec::new();
        for piece in batch.pieces().iter().filter(|p| want.contains(&p.font_size)) {
            match clusters.iter_mut().find(|seen| seen.intersects(piece.ink)) {
                Some(seen) => *seen = seen.union(piece.ink),
                None => clusters.push(piece.ink),
            }
        }
        clusters
    };
    // Letter and marks together: the readout has to clear the comma, which
    // hangs lower than the letter does.
    let names = ink_of(&[panes::lattice::NAME_SIZE, panes::lattice::MARK_SIZE]);
    let cents = ink_of(&[panes::lattice::CENTS_SIZE]);
    assert!(!names.is_empty() && !cents.is_empty(), "the held C should be labeled");

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

/// Drag the Spectral pane's spectrum/spectrogram divider through the REAL
/// dock — `root_ui`, egui_dock, the tab body's ScrollArea and all.
///
/// The pane's own tests drive `spectral_pane` into a bare child Ui, which
/// skips every layer the dock puts between the pointer and the handle. Any
/// of those could swallow the drag (the ScrollArea registers a drag-sensing
/// background widget of its own), and the failure would look exactly like
/// "dragging doesn't work" — silent, with the handle still lighting up on
/// hover. So the assertion is that the split actually MOVED.
#[test]
fn the_spectral_divider_drags_through_the_dock() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let backend = RecordingBackend::default();
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 800.0));
    let mut t = 0.0;
    let mut frame = |state: &mut SharedState, events: Vec<egui::Event>| {
        t += 1.0 / 60.0;
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(t),
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, |ui| root_ui(ui, state, &backend, t));
    };
    // Two warm-ups: egui resolves the top widget at the pointer from the
    // previous pass, so the handle has to exist before the press.
    frame(&mut state, vec![]);
    frame(&mut state, vec![]);

    // Ask egui where the handle actually landed rather than deriving the
    // dock's arithmetic here, which would just re-encode the layout.
    let handle = egui::Id::new(("spectral-split", 0usize));
    let band = ctx.read_response(handle).expect("the split handle never registered").rect;
    let grab = band.center();
    let before = state.spectrum_config.roll_fraction;

    // Across (the default orientation) puts the divider upright, so the drag
    // that moves it runs along x — pushing it away from the spectrum.
    let press = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    frame(&mut state, vec![egui::Event::PointerMoved(grab)]);
    assert!(
        ctx.read_response(handle).is_some_and(|r| r.hovered()),
        "the handle should light up under the pointer",
    );
    frame(&mut state, vec![egui::Event::PointerMoved(grab), press(grab, true)]);
    let target = grab + egui::vec2(40.0, 0.0);
    frame(&mut state, vec![egui::Event::PointerMoved(target)]);
    assert!(ctx.read_response(handle).is_some_and(|r| r.dragged()), "the handle should be dragged");
    frame(&mut state, vec![press(target, false)]);

    let after = state.spectrum_config.roll_fraction;
    assert!(
        after < before - 0.1,
        "the split should have moved with the pointer ({before} -> {after})",
    );
}

/// A harness that runs the REAL dock — `root_ui`, egui_dock, tab bodies and
/// all — one frame per call, so a pane's pointer handling is tested through
/// every layer that sits between it and the mouse.
struct DockHarness {
    ctx: egui::Context,
    backend: RecordingBackend,
    screen: egui::Rect,
    t: f64,
}

impl DockHarness {
    fn new() -> Self {
        DockHarness {
            ctx: egui::Context::default(),
            backend: RecordingBackend::default(),
            screen: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 800.0)),
            t: 0.0,
        }
    }

    fn frame(&mut self, state: &mut SharedState, events: Vec<egui::Event>) {
        self.t += 1.0 / 60.0;
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            time: Some(self.t),
            events,
            ..Default::default()
        };
        let t = self.t;
        let backend = &self.backend;
        let _ = self.ctx.run_ui(raw, |ui| root_ui(ui, state, backend, t));
    }

    /// Two warm-ups: egui resolves the top widget at the pointer from the
    /// previous pass, so a widget has to exist before the press.
    fn settle(&mut self, state: &mut SharedState) {
        self.frame(state, vec![]);
        self.frame(state, vec![]);
    }

    /// A point inside the Spectral pane's picture, mid-pitch and deep into the
    /// roll/spectrogram region — clear of the divider, which sits at 45% of
    /// the depth axis by default and would otherwise take the drag.
    fn spectral_grab(&self, state: &SharedState) -> egui::Pos2 {
        self.spectral_grab_at(state, 0.8)
    }

    /// The same, at a chosen fraction along the depth (time) axis — which side
    /// of the divider a drag starts on decides whether it is the Span's.
    /// Across, the default orientation, runs depth rightward.
    fn spectral_grab_at(&self, state: &SharedState, depth: f32) -> egui::Pos2 {
        // The perf overlay already answers "where is the Spectral pane's
        // body", and falls back to the whole window when it isn't on screen.
        let rect = perf_overlay_area(state, self.screen);
        assert_ne!(rect, self.screen, "the Spectral pane should be visible in the default dock");
        rect.lerp_inside(egui::vec2(depth, 0.5))
    }
}

/// Hovering the analyzer writes the matching lattice node into the shared
/// hover state, and follows the pointer up the pitch axis.
///
/// This is the whole of what the hover does now that it no longer also prints
/// the pitch beside the cursor, and nothing asserted it: several tests drive a
/// pointer into this pane for other reasons, so the write was executed
/// constantly and checked never — deleting it outright left the suite green.
///
/// Driven at the pane rather than through the dock, because the dock does not
/// leave the value observable: `lattice_pane` re-derives the scene from
/// `state.hovered` and then clears it when the pointer is not over ITS rect
/// (`panes::lattice`), so the analyzer's write is consumed inside the frame it
/// is made and reads as `None` by the end of one. The highlight is real; the
/// end-of-frame value is not where to look for it.
///
/// Tolerance is opened up deliberately. It ships at half a cent, against a
/// hovered pitch that is continuous, so at the default a node lights only
/// within a sub-pixel band of an exact semitone and a test aiming at one
/// would be asserting the pane's pixel geometry rather than its hover.
#[test]
fn hovering_the_analyzer_names_the_lattice_node_under_the_pointer() {
    let hovered_at = |frac: f32| {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        state.tuning = lattice_core::Tuning::from_cents(
            0.0,
            lattice_core::tuning::THREE_12TET,
            lattice_core::tuning::FIVE_12TET,
            lattice_core::tuning::SEVEN_12TET,
            50.0,
        );
        // One octave on the axis, so two points on it are two pitch CLASSES.
        // Across the default ten-octave range, 0.2 and 0.8 sit exactly six
        // octaves apart and reduce to the same class — which looks like a
        // hover that does not track and is really the range being wide.
        state.spectrum_config.low_midi = 60.0;
        state.spectrum_config.high_midi = 72.0;
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, 400.0));
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        // Twice: egui resolves the widget under the pointer from the previous
        // pass, so the pane has to exist before the hover lands on it.
        for _ in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(rect),
                events: vec![egui::Event::PointerMoved(rect.lerp_inside(egui::vec2(0.8, frac)))],
                ..Default::default()
            };
            let _ = ctx.run_ui(raw, |ui| {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                crate::panes::spectral::spectral_pane(&mut child, &mut state, 0.5, 1.0, 0);
            });
        }
        state.hovered
    };

    // Across is the default orientation, so pitch climbs UP the screen: two
    // points a good way apart vertically are two different pitches.
    let (low, high) = (hovered_at(0.8), hovered_at(0.2));
    assert!(low.is_some(), "hovering the analyzer should name a node, got None");
    assert!(high.is_some(), "hovering the analyzer should name a node, got None");
    assert_ne!(
        low, high,
        "the hover should follow the pointer along the pitch axis, not sit on one node",
    );
}

fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// Dragging the Spectral pane's picture pans the pitch range, through the real
/// dock. Panning DOWN the axis (dragging toward higher pitch) has to bring
/// lower pitches into view, the way grabbing any picture does.
#[test]
fn dragging_the_spectral_picture_pans_the_pitch_range() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    // Start zoomed in, so there is room to pan in both directions.
    state.spectrum_config.low_midi = 48.0;
    state.spectrum_config.high_midi = 84.0;
    let mut h = DockHarness::new();
    h.settle(&mut state);

    let grab = h.spectral_grab(&state);
    let before = state.spectrum_config;
    // Across (the default orientation) climbs in pitch UP the screen, so a
    // drag toward higher pitch is a drag toward smaller y.
    h.frame(&mut state, vec![egui::Event::PointerMoved(grab), press(grab, true)]);
    let target = grab + egui::vec2(0.0, -60.0);
    h.frame(&mut state, vec![egui::Event::PointerMoved(target)]);
    h.frame(&mut state, vec![press(target, false)]);

    let after = state.spectrum_config;
    assert!(
        after.low_midi < before.low_midi - 1.0,
        "the range should have followed the pointer down the axis ({} -> {})",
        before.low_midi,
        after.low_midi,
    );
    assert!(
        ((after.high_midi - after.low_midi) - (before.high_midi - before.low_midi)).abs() < 1e-3,
        "a pan moves the range without resizing it",
    );
    assert_eq!(
        after.roll_seconds, before.roll_seconds,
        "a drag across the pitch axis is the range's; the Span must not breathe with it",
    );
}

/// Dragging along the time axis zooms the roll's Span instead — the picture is
/// anchored at the now-line, so pulling it toward the past spreads it out and
/// the seconds it spans shrink. The pitch range stays where it was: one drag
/// moves one axis.
#[test]
fn dragging_the_spectral_picture_along_time_zooms_the_span() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let mut h = DockHarness::new();
    h.settle(&mut state);

    let grab = h.spectral_grab(&state);
    let before = state.spectrum_config;
    // Across runs time rightward (now at the left), so dragging right is
    // dragging toward the past.
    h.frame(&mut state, vec![egui::Event::PointerMoved(grab), press(grab, true)]);
    let target = grab + egui::vec2(120.0, 0.0);
    h.frame(&mut state, vec![egui::Event::PointerMoved(target)]);
    h.frame(&mut state, vec![press(target, false)]);

    let zoomed = state.spectrum_config;
    assert!(
        zoomed.roll_seconds < before.roll_seconds * 0.75,
        "dragging toward the past should have zoomed in ({} -> {})",
        before.roll_seconds,
        zoomed.roll_seconds,
    );
    assert_eq!(
        (zoomed.low_midi, zoomed.high_midi),
        (before.low_midi, before.high_midi),
        "the Span's drag is not the pitch range's",
    );

    // And back: the mapping is exponential in the drag, so the same distance
    // the other way returns the span it started on. Grabbed from the same spot
    // (the far end of a rightward drag can land outside the pane, where a press
    // is nobody's drag).
    let back = grab - egui::vec2(120.0, 0.0);
    h.frame(&mut state, vec![egui::Event::PointerMoved(grab), press(grab, true)]);
    h.frame(&mut state, vec![egui::Event::PointerMoved(back)]);
    h.frame(&mut state, vec![press(back, false)]);
    let restored = state.spectrum_config.roll_seconds;
    assert!(
        (restored - before.roll_seconds).abs() < 0.1,
        "dragging back should restore the span ({} -> {restored})",
        before.roll_seconds,
    );
}

/// Only a drag that starts in the far region — where the time axis actually is
/// — zooms the Span. Over the spectrum's own share the depth axis is dB, and a
/// drag along it would be moving something that isn't under the hand.
#[test]
fn a_drag_over_the_spectrum_leaves_the_span_alone() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    // Zoomed in, so the pan the drag DOES do has room to show — otherwise this
    // would pass just as well on a drag that never reached the pane.
    state.spectrum_config.low_midi = 48.0;
    state.spectrum_config.high_midi = 84.0;
    let mut h = DockHarness::new();
    h.settle(&mut state);

    // The spectrum owns 0..0.45 of the depth axis by default.
    let grab = h.spectral_grab_at(&state, 0.2);
    let before = state.spectrum_config;
    h.frame(&mut state, vec![egui::Event::PointerMoved(grab), press(grab, true)]);
    // Leaning along time, which in the far region would be a Span zoom.
    let target = grab + egui::vec2(120.0, -20.0);
    h.frame(&mut state, vec![egui::Event::PointerMoved(target)]);
    h.frame(&mut state, vec![press(target, false)]);

    let after = state.spectrum_config;
    assert_eq!(
        after.roll_seconds, before.roll_seconds,
        "a drag begun over the spectrum has no time axis under it",
    );
    assert!(
        after.low_midi < before.low_midi - 0.1,
        "and is still a pitch pan, so the drag did reach the pane ({} -> {})",
        before.low_midi,
        after.low_midi,
    );
}

/// The wheel zooms the pitch range, and touches NOTHING else — in particular
/// not the roll's time Span, which is the other thing a wheel over this pane
/// could plausibly have meant.
#[test]
fn the_wheel_zooms_the_pitch_range_and_leaves_the_time_span_alone() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.spectrum_config.low_midi = 36.0;
    state.spectrum_config.high_midi = 96.0;
    let mut h = DockHarness::new();
    h.settle(&mut state);

    let over = h.spectral_grab(&state);
    let before = state.spectrum_config;
    h.frame(&mut state, vec![egui::Event::PointerMoved(over)]);
    // Several notches, so the assertion isn't riding on egui's scroll smoothing
    // having fully caught up in one frame.
    for _ in 0..4 {
        h.frame(
            &mut state,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, 40.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::default(),
            }],
        );
    }

    let after = state.spectrum_config;
    let (was, now) = (before.high_midi - before.low_midi, after.high_midi - after.low_midi);
    assert!(now < was - 1.0, "scrolling up should have zoomed in ({was} -> {now})");
    assert_eq!(
        after.roll_seconds, before.roll_seconds,
        "the wheel is the pitch range's, not the time axis's",
    );
    // Zoom is anchored on the pointer, which is the middle of the pane here,
    // so the pitch under it should not have moved.
    let mid = |c: &crate::SpectrumConfig| 0.5 * (c.low_midi + c.high_midi);
    assert!(
        (mid(&after) - mid(&before)).abs() < 1.0,
        "the pitch under the pointer should stay put ({} -> {})",
        mid(&before),
        mid(&after),
    );
}

/// The pane now senses drags over its whole surface, which is exactly what
/// could have swallowed the divider's. It must not: the divider registers
/// after the pane, so egui leaves it on top, and a drag that starts on the
/// handle still resizes the split and does NOT pan the pitch.
#[test]
fn the_divider_still_wins_the_drag_over_the_pane_behind_it() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let mut h = DockHarness::new();
    h.settle(&mut state);

    let handle = egui::Id::new(("spectral-split", 0usize));
    let band = h.ctx.read_response(handle).expect("the split handle never registered").rect;
    let grab = band.center();
    let before = state.spectrum_config;

    h.frame(&mut state, vec![egui::Event::PointerMoved(grab), press(grab, true)]);
    // A drag with a pitch-axis component, so a pane that stole it would show
    // up as a moved range rather than as nothing happening.
    let target = grab + egui::vec2(40.0, -30.0);
    h.frame(&mut state, vec![egui::Event::PointerMoved(target)]);
    h.frame(&mut state, vec![press(target, false)]);

    let after = state.spectrum_config;
    assert!(
        (after.roll_fraction - before.roll_fraction).abs() > 0.01,
        "the divider should still have moved",
    );
    assert_eq!(
        (after.low_midi, after.high_midi),
        (before.low_midi, before.high_midi),
        "dragging the divider must not pan the pitch range as well",
    );
    assert_eq!(
        after.roll_seconds, before.roll_seconds,
        "nor zoom the Span — the drag leans along time, which is the Span's gesture",
    );
}

#[test]
fn frame_interval_converts_a_cap_to_a_spacing() {
    assert_eq!(frame_interval(None), None, "uncapped asks for no spacing");
    assert_eq!(
        frame_interval(Some(30.0)),
        Some(std::time::Duration::from_secs_f32(1.0 / 30.0)),
    );
    assert_eq!(
        frame_interval(Some(144.0)),
        Some(std::time::Duration::from_secs_f32(1.0 / 144.0)),
    );
}

#[test]
fn nonsense_caps_read_as_uncapped() {
    // The control cannot produce these, but a hand-edited persist blob can.
    // Uncapped is the safe reading: a zero interval is the uncapped
    // behaviour with extra steps, and a huge one would freeze the UI.
    for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(frame_interval(Some(bad)), None, "{bad} should read as uncapped");
    }
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

/// The heatmap's level for a bucket given as POWER. The pane itself reads the
/// dB its history already stores; these tests are about the mapping, which is
/// easier to state in the power a partial actually has.
fn spectrogram_level(cfg: &SpectrumConfig, power: f32, midi: f32) -> f32 {
    use crate::panes::spectral::{power_db, spectrogram_level_db};
    spectrogram_level_db(cfg, power_db(power), midi)
}

#[test]
fn the_heatmap_follows_the_curve_until_given_its_own_range() {
    use crate::panes::spectral::loudness;
    let mut cfg = SpectrumConfig::default();
    let (power, midi) = (1e-4, 60.0);

    // Shared by default — the behaviour every existing persist blob expects.
    assert_eq!(spectrogram_level(&cfg, power, midi), loudness(&cfg, power, midi));

    // Switched on, the heatmap reads its OWN window and stops tracking the
    // curve's: moving the curve's floor must not move the heatmap.
    cfg.spectrogram_own_range = true;
    let own = spectrogram_level(&cfg, power, midi);
    cfg.floor_db = -20.0;
    assert_eq!(spectrogram_level(&cfg, power, midi), own, "the heatmap tracked the curve");
    assert_ne!(loudness(&cfg, power, midi), own, "the curve should have moved");
}

#[test]
fn heatmap_contrast_bends_the_level_without_clipping_it() {
    let mut cfg = SpectrumConfig::default();
    // A power that lands mid-range, so there is room to bend either way.
    let (power, midi) = (1e-4, 60.0);
    let straight = spectrogram_level(&cfg, power, midi);
    assert!(straight > 0.01 && straight < 0.99, "need a mid-scale level, got {straight}");

    cfg.spectrogram_gamma = 0.5;
    let lifted = spectrogram_level(&cfg, power, midi);
    cfg.spectrogram_gamma = 2.5;
    let crushed = spectrogram_level(&cfg, power, midi);
    assert!(lifted > straight, "gamma below 1 should lift ({straight} -> {lifted})");
    assert!(crushed < straight, "gamma above 1 should crush ({straight} -> {crushed})");
    // The ends are fixed points: contrast redistributes, it never clips.
    for gamma in [0.3, 1.0, 3.0] {
        cfg.spectrogram_gamma = gamma;
        assert_eq!(spectrogram_level(&cfg, 0.0, midi), 0.0, "silence moved at gamma {gamma}");
        assert_eq!(spectrogram_level(&cfg, 1e9, midi), 1.0, "full scale moved at gamma {gamma}");
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

/// Drive the REAL dock (root_ui, egui_dock, the tab body's ScrollArea and
/// all) with a wheel over `tab`'s body, and answer how far its content moved.
/// Negative = the content moved up, i.e. the pane scrolled down.
///
/// Tracks NAMED texts rather than a bounding box: egui culls whatever scrolls
/// out of the clip rect and the custom bars paint past it, so every
/// position-of-the-ink metric reports movement that isn't there (and misses
/// movement that is). The y of a string drawn in both frames cannot lie.
fn wheel_over_settings_pane(tab: panes::Tab, screen_h: f32) -> f32 {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    // The settings leaf opens on Tuning; every other settings pane is a tab
    // behind it.
    let path = state.dock.find_tab(&tab).expect("{tab:?} is not in the default dock");
    state.dock.set_active_tab(path).expect("selecting the tab");
    let backend = RecordingBackend::default();
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, screen_h));
    // The top-right leaf (right of the 0.72 split, above the 0.55 one), from
    // under its tab bar down. Only shapes clipped to this are the pane's.
    let body = egui::Rect::from_min_max(
        egui::pos2(700.0, 20.0),
        egui::pos2(1000.0, screen_h * 0.55 + 2.0),
    );
    let texts = |out: &egui::FullOutput| {
        let mut map = std::collections::HashMap::new();
        for cs in &out.shapes {
            if cs.clip_rect.min.x < body.min.x
                || cs.clip_rect.min.y < body.min.y
                || cs.clip_rect.max.y > body.max.y
            {
                continue;
            }
            if let egui::Shape::Text(t) = &cs.shape {
                map.entry(t.galley.text().to_owned()).or_insert(t.pos.y);
            }
        }
        map
    };
    let mut t = 0.0;
    let mut frame = |state: &mut SharedState, events: Vec<egui::Event>| {
        t += 1.0 / 60.0;
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(t),
            events,
            ..Default::default()
        };
        texts(&ctx.run_ui(raw, |ui| root_ui(ui, state, &backend, t)))
    };
    // egui resolves the widget under the pointer from the previous pass, so
    // the pointer has to be there for a frame before the wheel arrives.
    frame(&mut state, vec![egui::Event::PointerMoved(egui::pos2(860.0, screen_h * 0.22))]);
    let before = frame(&mut state, vec![]);
    frame(
        &mut state,
        vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -3.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    // The wheel arrives smoothed over several frames.
    let mut after = before.clone();
    for _ in 0..20 {
        after = frame(&mut state, vec![]);
    }
    let mut deltas: Vec<f32> = before
        .iter()
        .filter_map(|(text, y)| after.get(text).map(|moved| moved - y))
        .collect();
    assert!(!deltas.is_empty(), "{tab:?} drew no text to measure");
    deltas.sort_by(f32::total_cmp);
    deltas[deltas.len() / 2]
}

/// Every settings pane scrolls to the wheel once its content is taller than
/// the pane. The dock hands some of them its own `ScrollArea` and others build
/// their own; from the wheel's side that must not be visible.
#[test]
fn every_settings_pane_scrolls_when_its_content_overflows() {
    // A short window, so that every one of them overflows — including Panel,
    // the shortest list of the set.
    for tab in [
        panes::Tab::Tuning,
        panes::Tab::Nodes,
        panes::Tab::Scene,
        panes::Tab::Analyzer,
        panes::Tab::Video,
        panes::Tab::Panel,
    ] {
        let moved = wheel_over_settings_pane(tab, 300.0);
        assert!(moved < -8.0, "{tab:?} did not scroll to the wheel (content moved {moved})");
    }
}

/// A drag whose release never arrives must not take the wheel down with it.
///
/// egui gates every `ScrollArea` on `dragged_id().is_none()` — globally, not
/// per area — so one stale drag stops the wheel in EVERY settings pane at once.
/// In a plugin window that is a routine event rather than an exotic one: let go
/// outside the editor, or let the host take focus mid-drag, and the release is
/// delivered somewhere that is not us. Both gestures below are ones a person
/// actually makes: panning the Analyzer's pitch range out past its edge, and
/// dragging any settings bar.
#[test]
fn a_drag_that_loses_its_release_does_not_strand_the_wheel() {
    // Default dock: the Analyzer picture is the column at x ~518..720, the
    // settings leaf is top-right.
    for (what, at) in [("the analyzer picture", 600.0f32), ("a settings bar", 860.0)] {
        for lose_it in [Lose::Pointer, Lose::Focus] {
            let moved = scroll_settings_after_lost_drag(egui::pos2(at, 200.0), lose_it);
            assert!(
                moved < -8.0,
                "a drag on {what} that lost its release to {lose_it:?} left the settings \
                 pane unscrollable (content moved {moved})",
            );
        }
    }
}

/// How the release goes missing: the pointer leaves the editor, or the host
/// takes focus while the button is down.
#[derive(Clone, Copy, Debug)]
enum Lose {
    Pointer,
    Focus,
}

/// Press and drag at `from`, lose the release, then wheel over the settings
/// pane and answer how far its content moved.
fn scroll_settings_after_lost_drag(from: egui::Pos2, lose: Lose) -> f32 {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let path = state.dock.find_tab(&panes::Tab::Analyzer).expect("the Analyzer settings tab");
    state.dock.set_active_tab(path).expect("selecting the tab");
    let backend = RecordingBackend::default();
    let ctx = egui::Context::default();
    let screen_h = 500.0;
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, screen_h));
    let body =
        egui::Rect::from_min_max(egui::pos2(700.0, 20.0), egui::pos2(1000.0, screen_h * 0.55 + 2.0));
    // Named texts inside the settings body, as `wheel_over_settings_pane` does:
    // the y of a string drawn in both frames is the one metric a clip rect and
    // a culled shape cannot lie about.
    let texts = |out: &egui::FullOutput| {
        let mut map = std::collections::HashMap::new();
        for cs in &out.shapes {
            if cs.clip_rect.min.x < body.min.x
                || cs.clip_rect.min.y < body.min.y
                || cs.clip_rect.max.y > body.max.y
            {
                continue;
            }
            if let egui::Shape::Text(t) = &cs.shape {
                map.entry(t.galley.text().to_owned()).or_insert(t.pos.y);
            }
        }
        map
    };
    let mut t = 0.0;
    let mut frame = |state: &mut SharedState, events: Vec<egui::Event>| {
        t += 1.0 / 60.0;
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(t),
            events,
            ..Default::default()
        };
        texts(&ctx.run_ui(raw, |ui| root_ui(ui, state, &backend, t)))
    };
    let press = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    // Hover, press, drag — and then the release goes missing.
    frame(&mut state, vec![egui::Event::PointerMoved(from)]);
    frame(&mut state, vec![press(from, true)]);
    frame(&mut state, vec![egui::Event::PointerMoved(from + egui::vec2(0.0, 40.0))]);
    frame(
        &mut state,
        match lose {
            Lose::Pointer => vec![egui::Event::PointerGone],
            Lose::Focus => vec![egui::Event::WindowFocused(false)],
        },
    );

    // Back over the settings pane, wheel, and see whether anything moves.
    let settings = egui::pos2(860.0, 130.0);
    frame(&mut state, vec![egui::Event::PointerMoved(settings)]);
    let before = frame(&mut state, vec![]);
    frame(
        &mut state,
        vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -3.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    // The wheel arrives smoothed over several frames.
    let mut after = before.clone();
    for _ in 0..20 {
        after = frame(&mut state, vec![]);
    }
    let mut deltas: Vec<f32> =
        before.iter().filter_map(|(text, y)| after.get(text).map(|m| m - y)).collect();
    assert!(!deltas.is_empty(), "the settings pane drew no text to measure");
    deltas.sort_by(f32::total_cmp);
    deltas[deltas.len() / 2]
}

/// The Video pane scrolls at a workable size, rather than swallowing the slack
/// with its preview.
///
/// It was the one settings pane the wheel did nothing in, at every size a
/// person would actually use. The preview took `available_size()`, so the
/// pane's content measured *exactly* the pane however short the pane got —
/// the dock's `ScrollArea` never saw anything sticking out to scroll, and the
/// preview shrank towards a sliver instead of the controls staying reachable.
#[test]
fn the_video_pane_scrolls_instead_of_squeezing_its_preview() {
    let moved = wheel_over_settings_pane(panes::Tab::Video, 600.0);
    assert!(moved < -8.0, "the Video pane did not scroll to the wheel (content moved {moved})");
}

/// The performance overlay hangs off the analyzer pane, and falls back to the
/// whole editor when that pane isn't the one on screen.
#[test]
fn the_perf_overlay_follows_the_analyzer_pane() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let backend = RecordingBackend::default();
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 800.0));
    let mut t = 0.0;
    let mut frame = |state: &mut SharedState| {
        t += 1.0 / 60.0;
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(t),
            ..Default::default()
        };
        ctx.run_ui(raw, |ui| root_ui(ui, state, &backend, t))
    };
    // A frame first: the dock only knows where its panes are once it has laid
    // them out, and before that the overlay has nothing to hang off.
    frame(&mut state);
    let output = frame(&mut state);

    let area = perf_overlay_area(&state, screen);
    assert_ne!(area, screen, "the overlay should have found the analyzer pane");

    // ...and the HUD really lands in that pane's top-right corner.
    //
    // Found by its painted text rather than by `Memory::area_rect`: the HUD is
    // not an Area. As one, every label inside it registers a widget rect that
    // takes the pointer from whatever is underneath — a dead zone the size of
    // the readout. It is painted straight onto a foreground layer, so there is
    // no area to look up, and the thing worth asserting was never the Area
    // anyway: it is where the numbers land.
    assert!(
        output.shapes.iter().any(|clipped| matches!(
            &clipped.shape,
            egui::Shape::Text(text) if text.galley.text().contains("fps")
        )),
        "the overlay should be drawn (show_perf is on by default)",
    );
    // The backing plate, which is the HUD's actual extent — the rows inside it
    // are left-aligned, so no single string reveals where the box sits.
    let plate = egui::Color32::from_black_alpha(0xC0);
    let hud = output
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) if rect.fill == plate => Some(rect.rect),
            _ => None,
        })
        .expect("the overlay should paint its backing plate");
    assert!(area.contains_rect(hud), "the HUD should sit inside the analyzer pane: {hud:?}");
    assert!(
        (hud.right() - (area.right() - 8.0)).abs() < 1.0,
        "the HUD should hug the pane's RIGHT edge: {hud:?} in {area:?}",
    );
    assert!(
        (hud.right() - area.right()).abs() < 12.0 && (hud.top() - area.top()).abs() < 12.0,
        "the HUD should hug the pane's top-RIGHT corner: {hud:?} in {area:?}",
    );
    // The build tag, which is why the HUD is worth looking at before any of
    // its numbers: Bitwig loads ONE bundle and every session builds into its
    // own worktree, so "am I even looking at the build I just loaded?" has a
    // wrong answer available. Asserted as painted TEXT, because a tag that is
    // computed and not drawn would verify nothing.
    //
    // Wrapping means the tag can span two galleys, so this looks for the
    // branch name rather than the whole line.
    let branch = perf::BUILD_TAG.split(" @").next().unwrap_or(perf::BUILD_TAG);
    assert!(
        output.shapes.iter().any(|clipped| matches!(
            &clipped.shape,
            egui::Shape::Text(text) if text.galley.text().contains(branch)
        )),
        "the overlay should name the build it is ({}), so a reload can be checked",
        perf::BUILD_TAG,
    );
    // ...and naming it must not have pushed the HUD out of its pane. The tag
    // is a branch name, so it is arbitrarily long; `draw_overlay` wraps it to
    // the width the numbers already need. Without that, a long enough branch
    // silently widens the HUD past the pane — which the assertion above on
    // `contains_rect` catches, but only on a branch that happens to be long.
    assert!(
        hud.width() < area.width(),
        "the build tag must wrap, not widen the HUD: {hud:?} in {area:?}",
    );

    assert!(screen.contains_rect(area), "the analyzer pane is inside the editor");
    // Right of the lattice and left of the settings column: the Spectral pane
    // as `default_dock` places it.
    assert!(area.left() > screen.left(), "the analyzer pane is not the left edge");
    assert!(area.right() < screen.right(), "the settings column is right of it");

    // Its leaf holds Spectral alone, so collapsing it is what takes it off
    // screen; the overlay then falls back to the whole editor.
    let path = state.dock.find_tab(&panes::Tab::Spectral).expect("Spectral is docked");
    let egui_dock::Node::Leaf(leaf) = &mut state.dock[path.surface][path.node] else {
        panic!("Spectral should live in a leaf");
    };
    leaf.collapsed = true;
    assert_eq!(
        perf_overlay_area(&state, screen),
        screen,
        "a collapsed analyzer pane should hand the overlay back to the editor",
    );
}


/// A key this build has RETIRED does not cost the blob it sits in.
///
/// `load_persist` takes the whole `UiPersist` or nothing (`if let Ok(persist)`),
/// so a field that fails to parse does not degrade — it silently discards the
/// dock, the camera and the entire `ViewConfig` along with itself, and the
/// project opens on defaults with no error anywhere. Retiring a setting is
/// therefore a persistence change, and this is the guard on it: `roll_gap` was
/// removed with the Gap feature, and every project saved before that still
/// carries the key.
#[test]
fn a_retired_setting_does_not_discard_the_blob_it_was_saved_in() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.spectrum_config.roll_thickness = 1.75;
    state.spectrum_config.low_midi = 40.5;
    let saved = state.save_persist();
    // A blob from before the retirement: the key spliced back where it sat.
    let old = saved.replacen("roll_thickness:", "roll_gap:2.5,roll_thickness:", 1);
    assert_ne!(old, saved, "the splice must have landed for this to test anything");

    let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
    restored.load_persist(&old);
    assert_eq!(restored.spectrum_config.roll_thickness, 1.75, "the blob survived");
    assert_eq!(restored.spectrum_config.low_midi, 40.5);
}
