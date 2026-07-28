//! Unit tests for the UI shell.

use super::*;
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
    state.view.outer_backdrop = 0.62;
    state.view.outer_solidity = 0.3;
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
    assert_eq!(restored.view.outer_backdrop, 0.62);
    assert_eq!(restored.view.outer_solidity, 0.3);
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
        let frame = crate::render_frame_from_persist(&saved).expect("still parses");
        assert_eq!(frame.lattice, side, "stacked:{flag} through render_frame_from_persist");
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

/// The layout opens with Notes and Console folded to their tab bar, and with
/// nothing else folded.
///
/// Both are read on demand — Notes restates what the lattice is already
/// drawing, Console is a diagnostic — and open they take 45% of the settings
/// column, the half the settings themselves want. The collapse arrow on the
/// folded bar brings either back at the size it went away, so this is a
/// starting point rather than a decision taken away.
#[test]
fn the_default_layout_opens_with_the_two_readout_panes_folded() {
    let dock = default_dock();
    let folded = |tab: panes::Tab| {
        let path = dock.find_tab(&tab).expect("docked by default");
        let egui_dock::Node::Leaf(leaf) = &dock[path.surface][path.node] else {
            panic!("{tab:?} should live in a leaf");
        };
        leaf.collapsed
    };
    assert!(folded(panes::Tab::Notes), "Notes should open folded");
    assert!(folded(panes::Tab::Console), "Console should open folded");
    for tab in [panes::Tab::Lattice, panes::Tab::Spectral, panes::Tab::Tuning] {
        assert!(!folded(tab), "{tab:?} should open on screen");
    }
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
        harmonigraph_scene::LegacyNodeBody::Disc,
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
        state.tracker.handle_event(harmonigraph_core::NoteEvent {
            time: 0.0,
            channel: 0,
            note,
            kind: harmonigraph_core::NoteEventKind::On { velocity: 1.0 },
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
        state.tracker.handle_event(harmonigraph_core::NoteEvent {
            time: 0.0,
            channel: 0,
            note,
            kind: harmonigraph_core::NoteEventKind::On { velocity: 1.0 },
        });
        if cents != 0.0 {
            state.tracker.handle_event(harmonigraph_core::NoteEvent {
                time: 0.0,
                channel: 0,
                note,
                kind: harmonigraph_core::NoteEventKind::Tuning { semitones: cents / 100.0 },
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
    let just_offset = harmonigraph_core::tuning::FIVE_JUST - 400.0;
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
    let a4 = ((69.0 - harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI)
        * harmonigraph_core::spectrum::BINS_PER_SEMITONE as f32) as i32;
    assert!((peak - a4).abs() <= 1, "440 Hz should peak at A4 (bucket {a4}), got {peak}");

    // Once samples stop, the curve hides instead of freezing.
    assert!(spectrum.display(1.0 + AudioSpectrum::HOLD_SECONDS + 0.1).is_none());
}

/// Music fills most of the analyzer's height, rather than half of it.
///
/// The ceiling used to be full scale, and nothing musical puts full scale in
/// ONE bucket: a chord splits its power across its partials, and the default
/// tilt takes another 10 dB off anything well under the 1 kHz pivot. The curve
/// topped out halfway up and the top half of the pane was empty in normal use.
///
/// So the defaults are held to a chord rather than to a test tone. This one
/// reads 0.90 of the pane as they stand and 0.60 against a full-scale ceiling,
/// so 0.75 is the line between the two — what it catches is the ceiling
/// drifting back up, not a shift of a few dB either way. The upper bound is
/// the other failure: a curve clipped flat against the top has lost the shape
/// of its own peaks, which is worse than empty space above it.
#[test]
fn a_chord_fills_most_of_the_analyzers_height() {
    let sr = 48_000.0;
    let cfg = SpectrumConfig::default();
    // Six partials sharing the headroom, peaking about -12 dBFS — a mix, not a
    // tone. Two seconds, so the smoothing has long settled.
    let samples: Vec<f32> = (0..24_000)
        .map(|i| {
            let t = i as f32 / sr;
            let mix: f32 = [220.0, 277.2, 329.6, 440.0, 554.4, 659.3]
                .iter()
                .map(|f| (std::f32::consts::TAU * f * t).sin())
                .sum();
            0.25 * mix / 6.0_f32.sqrt()
        })
        .collect();
    let mut spectrum = AudioSpectrum::default();
    spectrum.push_samples(&samples, 1, sr, 1.0, &cfg);
    let levels = spectrum.display(1.0).expect("audio is flowing");

    // The drawn height of the tallest bucket, through the same mapping the
    // curve is painted with — bucket index back to MIDI, since the tilt is a
    // function of pitch.
    let peak = levels
        .iter()
        .enumerate()
        .map(|(i, &power)| {
            let midi = harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI
                + i as f32 / harmonigraph_core::spectrum::BINS_PER_SEMITONE as f32;
            crate::panes::spectral::loudness(&cfg, power, midi)
        })
        .fold(0.0_f32, f32::max);

    assert!(peak > 0.75, "the curve only reaches {peak:.2} of the pane; the top is empty");
    assert!(peak < 0.99, "the curve is clipped flat against the ceiling at {peak:.2}");
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
fn spectrogram_history_stays_bounded() {
    let bins = [0.0f32; harmonigraph_core::spectrum::SPECTRUM_BINS];

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
    use harmonigraph_core::spectrum::midi_to_hz;
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
        ((harmonigraph_core::spectrum::hz_to_midi(hz) - harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI)
            * harmonigraph_core::spectrum::BINS_PER_SEMITONE as f32)
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
    use harmonigraph_core::spectrum::midi_to_hz;
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
    use harmonigraph_core::spectrogram::{db_of, DB_STEP};
    let a4 = ((69.0 - harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI)
        * harmonigraph_core::spectrum::BINS_PER_SEMITONE as f32)
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
    use harmonigraph_core::spectrum::{midi_to_hz, BINS_PER_SEMITONE, SPECTRUM_BINS, SPECTRUM_MIN_MIDI};
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

/// A label's text pieces AND the boxes of its drawn marks.
///
/// The comma signs are geometry rather than type (see
/// [`panes::lattice::draw_stacked_name`]), so a text-only view of a label is
/// blind to exactly the marks these tests are about. Each drawn piece emits
/// two shapes, halo then fill, and both are symmetric about the piece, so a
/// box here is the piece's own box grown by the halo.
fn drawn_label(
    name: harmonigraph_core::NoteName,
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
    let name = harmonigraph_core::NoteName {
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
        let name = harmonigraph_core::NoteName {
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
        harmonigraph_core::NoteName {
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
    let name = harmonigraph_core::NoteName {
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
    let name = harmonigraph_core::NoteName {
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
    let name = harmonigraph_core::NoteName {
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
    state.tracker.handle_event(harmonigraph_core::NoteEvent {
        time: 0.0,
        channel: 0,
        note: 60,
        kind: harmonigraph_core::NoteEventKind::On { velocity: 1.0 },
    });
    let scene = harmonigraph_scene::derive_scene(
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
        |ui| panes::lattice::draw_node_labels(ui, rect, &scene, &state.view, &mut batch),
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
    // Every size here scales with the pane and the camera together, so read
    // that scale off the biggest piece drawn rather than assuming the pane is
    // the one the constants are quoted at.
    let scale = batch.pieces().iter().map(|p| p.font_size).fold(0.0, f32::max)
        / panes::lattice::NAME_SIZE;
    // Letter and marks together: the readout has to clear the comma, which
    // hangs lower than the letter does.
    let names = ink_of(&[panes::lattice::NAME_SIZE * scale, panes::lattice::MARK_SIZE * scale]);
    let cents = ink_of(&[panes::lattice::CENTS_SIZE * scale]);
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
        // Slack enough for a DRAWN comma hanging below the letter's ink: the
        // readout clears it (`draw_stacked_name` reports it) but the clusters
        // above cannot see it, a drawn mark being a bitmap rather than a
        // glyph. It is a fraction of the type, so the slack is quoted against
        // the gap rather than in absolute points — the regression this is
        // watching for, hanging the readout off the galley box instead of the
        // ink, is worth twice the gap and clears any of this.
        let want = panes::lattice::CENTS_GAP * scale;
        let slack = want / 3.0;
        assert!(
            (gap - want).abs() <= slack,
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

    // Left (the default orientation) puts the divider upright, so the drag
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

    fn frame(&mut self, state: &mut SharedState, events: Vec<egui::Event>) -> egui::FullOutput {
        self.t += 1.0 / 60.0;
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            time: Some(self.t),
            events,
            ..Default::default()
        };
        let t = self.t;
        let backend = &self.backend;
        self.ctx.run_ui(raw, |ui| root_ui(ui, state, backend, t))
    }

    /// Answer a sideways fold's resize the way a shell does — the window it
    /// asked for, never below the floor it holds (see `fold`). Without this
    /// the harness is a host that refuses every resize, which is a state the
    /// fold layout has its own handling for.
    fn resize(&mut self, state: &mut SharedState) {
        if let Some(change) = state.take_window_width_change() {
            let width = (self.screen.width() + change).max(state.min_window_width);
            self.screen.max.x = self.screen.min.x + width;
        }
    }

    /// Frames until a fold has the window it asked for. A fold is a two-step —
    /// the frame that asks and the frame drawn at the size it was given — and
    /// one fold can release another, so this runs a few.
    fn settle_folds(&mut self, state: &mut SharedState) -> egui::FullOutput {
        let mut output = None;
        for _ in 0..4 {
            self.resize(state);
            output = Some(self.frame(state, vec![]));
        }
        output.expect("a settled frame")
    }

    /// A click on the collapse arrow of the leaf holding `tab`, settled.
    ///
    /// The ARROW, not the tab name: egui_dock reaches `set_collapsed` from its
    /// own square at the left end of the tab bar, and clicking the title only
    /// selects a tab.
    fn collapse_click(&mut self, state: &mut SharedState, tab: panes::Tab) -> egui::FullOutput {
        let path = state.dock.find_tab(&tab).expect("tab is in the dock");
        let rect = state.dock[path.surface][path.node].rect().expect("the leaf is laid out");
        let at = rect.left_top() + egui::vec2(12.0, crate::theme::TAB_BAR_HEIGHT * 0.5);
        self.frame(state, vec![egui::Event::PointerMoved(at)]);
        self.frame(state, vec![egui::Event::PointerMoved(at), press(at, true)]);
        self.frame(state, vec![press(at, false)]);
        self.settle_folds(state)
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
    /// Left, the default orientation, runs depth rightward.
    fn spectral_grab_at(&self, state: &SharedState, depth: f32) -> egui::Pos2 {
        // Asked of the Spectral pane BY NAME, so a dock that has taken it off
        // screen trips this rather than aiming the drag somewhere else.
        // `perf_overlay_area` answers the same question and is the wrong
        // oracle for it: it now falls back to the Lattice pane's body, which
        // is a perfectly good rect that a drag orbits the camera in — the
        // grab would land in the wrong pane and the test would fail three
        // asserts later, naming the analyzer.
        let rect = crate::pane_body(state, &panes::Tab::Spectral)
            .expect("the Spectral pane should be visible in the default dock");
        rect.lerp_inside(egui::vec2(depth, 0.5))
    }
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
    // Left (the default orientation) climbs in pitch UP the screen, so a
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
    // Left runs time rightward (now at the left), so dragging right is
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

/// The heatmap and the curve read ONE level scale, so a bucket that draws the
/// curve half way up paints a cell half way along the ramp, and dragging the
/// Level window moves both together.
///
/// A private dB window and a contrast curve on the heatmap's side would let the
/// same bucket mean two different things in one pane. Nothing enforces the
/// agreement but there being one mapping, and what holds THAT is comparing the
/// two ends: `loudness`, which the curve's height comes from, against
/// `bin_level`, which is what the heatmap's pixels actually go through.
///
/// Comparing `loudness` against `loudness_db(power_db(..))` instead proves
/// nothing whatever — that is `loudness`' own body, so both sides of the
/// assertion are one expression and no change to the heatmap can fail it. The
/// bridge has to be a function only the heatmap calls.
///
/// The tolerance is the store's, not the mapping's: `bin_level` reads a bucket
/// quantized to a byte of dB, so the two agree to within half a step of that
/// grid. `quantizing_a_bucket_does_not_move_its_colour` is where the step
/// itself is held.
#[test]
fn the_heatmap_reads_the_curve_s_own_level_scale() {
    use crate::panes::spectral::loudness;
    let mut cfg = SpectrumConfig::default();
    let midi = 60.0;
    let check = |cfg: &SpectrumConfig, power: f32| {
        let tolerance =
            0.5 * harmonigraph_core::spectrogram::DB_STEP / (cfg.ceiling_db - cfg.floor_db) + 1e-6;
        let curve = loudness(cfg, power, midi);
        let heatmap = crate::panes::spectrogram::bin_level_for_test(
            cfg,
            harmonigraph_core::spectrogram::quantize(power),
            midi,
        );
        assert!(
            (heatmap - curve).abs() <= tolerance,
            "power {power}: the curve reads {curve}, the heatmap {heatmap}",
        );
    };

    for power in [0.0, 1e-8, 1e-4, 1e-2, 1.0, 1e9] {
        check(&cfg, power);
    }
    // And they stay together as the window is dragged, at either end.
    cfg.floor_db = -20.0;
    cfg.ceiling_db = 0.0;
    check(&cfg, 1e-4);
    cfg.floor_db = -90.0;
    cfg.ceiling_db = -30.0;
    check(&cfg, 1e-6);
    // The tilt is the one input that makes the mapping pitch-dependent, so the
    // two have to track each other across pitch as well as across level.
    cfg.tilt = -6.0;
    for midi in [30.0f32, 60.0, 120.0] {
        let tolerance =
            0.5 * harmonigraph_core::spectrogram::DB_STEP / (cfg.ceiling_db - cfg.floor_db) + 1e-6;
        let curve = loudness(&cfg, 1e-5, midi);
        let heatmap = crate::panes::spectrogram::bin_level_for_test(
            &cfg,
            harmonigraph_core::spectrogram::quantize(1e-5),
            midi,
        );
        assert!((heatmap - curve).abs() <= tolerance, "MIDI {midi}: {curve} vs {heatmap}");
    }
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

/// The width a sideways fold gives up reaches the shell, through the whole
/// path the editor uses: `root_ui` measures the dock area from THIS frame's
/// `Ui`, the fold takes the pane's width out of it, and what is left over is
/// banked for the shell to spend on the window.
///
/// Driven through the real dock because that is the only place the area is
/// read from a live `Ui` rather than from the tree — `fold`'s own tests hand
/// `apply` a width directly, so a `root_ui` that measured the wrong rectangle,
/// or never asked at all, would leave every one of them green.
#[test]
fn collapsing_a_pane_banks_the_width_it_gave_up() {
    let mut h = DockHarness::new();
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    h.settle(&mut state);
    assert_eq!(state.take_window_width_change(), None, "an idle frame asks for nothing");

    let path = state.dock.find_tab(&panes::Tab::Lattice).expect("the lattice is docked");
    let pane = state.dock[path.surface][path.node].rect().expect("laid out").width();
    state.dock[path.surface][path.node].set_collapsed(true);
    h.frame(&mut state, vec![]);

    // A rail's worth of the pane stays behind; the rest comes off the window.
    let rail = theme::dock_style(&egui::Style::default()).tab_bar.height;
    let change = state.take_window_width_change().expect("the fold asks for a narrower window");
    assert!(
        (change + (pane - rail)).abs() < 1.0,
        "a {pane}pt pane leaving a {rail}pt rail should ask for {}, asked {change}",
        rail - pane,
    );
    h.frame(&mut state, vec![]);
    assert_eq!(state.take_window_width_change(), None, "and asks exactly once");
}

/// Put the Notes/Console leaf back on screen, which is what the two wheel
/// harnesses below are written against: they read the settings leaf as the box
/// from the tab bar down to the 0.55 split, and the default layout opens that
/// leaf folded (see
/// [`the_default_layout_opens_with_the_two_readout_panes_folded`]) so the
/// settings column runs the whole height instead.
///
/// Unfolded rather than measured where it now is, because a taller pane is the
/// wrong pane to ask these questions of: both tests need content that
/// OVERFLOWS, and the short window they pick is short relative to this box.
fn unfold_the_readout_panes(state: &mut SharedState) {
    let path = state.dock.find_tab(&panes::Tab::Notes).expect("Notes is docked");
    state.dock[path.surface][path.node].set_collapsed(false);
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
    unfold_the_readout_panes(&mut state);
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

/// The window the plugin is dialled in for, and the layout it opens with,
/// between them leave the settings column with NO scroll bar of either kind:
/// not the tab bar's, when the six tab names are laid across it, and not a
/// pane's own, when its controls are stacked down it.
///
/// A scroll bar there is a scroll bar over the controls, which is the one place
/// in the window that is nothing but controls — so it reads as the settings not
/// fitting the plugin rather than as a list being long. Both halves are tight
/// enough to lose by accident: the tab bar clears its content by 76pt of the
/// 423 it gets, and the tallest pane (the Analyzer's) only stopped overflowing
/// when the Notes/Console leaf folded and handed the column the other half of
/// its height. Add a settings tab, or unfold that leaf, and one of them comes
/// back.
///
/// 1512x886 because that is the window the sizes in this UI were chosen
/// against (see `panes::lattice::REFERENCE_HEIGHT`) — this says the defaults
/// agree with each other there, not that they survive every window. Narrower
/// than about 1240 and the tab bar does overflow, which is what its own scroll
/// bar is for.
#[test]
fn the_settings_column_needs_no_scroll_bar_at_the_window_it_was_dialled_in() {
    const REFERENCE: egui::Vec2 = egui::vec2(1512.0, 886.0);
    // Left edge of the settings column: everything right of the split.
    let column_left = REFERENCE.x * crate::state::SETTINGS_SPLIT;

    for tab in [
        panes::Tab::Tuning,
        panes::Tab::Nodes,
        panes::Tab::Scene,
        panes::Tab::Analyzer,
        panes::Tab::Video,
        panes::Tab::Panel,
    ] {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        let path = state.dock.find_tab(&tab).expect("a settings tab");
        state.dock.set_active_tab(path).expect("selecting the tab");
        let backend = RecordingBackend::default();
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, REFERENCE);

        // Named texts in the column, as the wheel harnesses above track them.
        let texts = |out: &egui::FullOutput| {
            let mut map = std::collections::HashMap::new();
            for cs in &out.shapes {
                if cs.clip_rect.min.x < column_left {
                    continue;
                }
                if let egui::Shape::Text(t) = &cs.shape {
                    map.entry(t.galley.text().to_owned()).or_insert(t.pos.y);
                }
            }
            map
        };
        // egui_dock draws its tab-bar scroll bar as a 7.5pt-tall rect, and only
        // when the tabs overflow — so finding one IS the overflow.
        let scroll_bars = |out: &egui::FullOutput| {
            out.shapes
                .iter()
                .filter(|cs| match &cs.shape {
                    egui::Shape::Rect(r) => {
                        (r.rect.height() - 7.5).abs() < 0.01 && r.rect.min.x >= column_left
                    }
                    _ => false,
                })
                .count()
        };

        let mut t = 0.0;
        // Each frame answers with both readings: the named texts, and how many
        // tab-bar scroll bars the column drew.
        let mut frame = |state: &mut SharedState, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(t),
                    events,
                    ..Default::default()
                },
                |ui| root_ui(ui, state, &backend, t),
            );
            (texts(&out), scroll_bars(&out))
        };

        // The pointer has to sit over the pane for a frame before the wheel
        // lands, since egui resolves it from the previous pass.
        frame(&mut state, vec![egui::Event::PointerMoved(egui::pos2(1250.0, 300.0))]);
        let (before, bars) = frame(&mut state, vec![]);
        assert_eq!(bars, 0, "{tab:?}: the tab bar drew a scroll bar at {REFERENCE:?}");
        frame(
            &mut state,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, -3.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let mut after = before.clone();
        for _ in 0..20 {
            after = frame(&mut state, vec![]).0;
        }
        let mut deltas: Vec<f32> =
            before.iter().filter_map(|(text, y)| after.get(text).map(|m| m - y)).collect();
        assert!(!deltas.is_empty(), "{tab:?} drew no text to measure");
        deltas.sort_by(f32::total_cmp);
        let moved = deltas[deltas.len() / 2];
        assert_eq!(moved, 0.0, "{tab:?} still scrolls at {REFERENCE:?} (content moved {moved})");
    }
}

/// The projections a settings sweep has to cover, default first.
///
/// Only the Tuning pane's content turns on this, and it turns on it hard:
/// `frame_controls` hides the whole camera-angle half — Camera yaw and pitch,
/// the Angle presets, the Save-angle row — under Cabinet, which has a fixed
/// viewpoint and no angle to set, and hides the two cabinet knobs under the
/// others. `Camera::default()` IS Cabinet, so a fixture that takes the default
/// and stops there never draws that half of the pane at all.
const PROJECTIONS: [harmonigraph_scene::Projection; 3] = [
    harmonigraph_scene::Projection::Cabinet,
    harmonigraph_scene::Projection::Perspective,
    harmonigraph_scene::Projection::Orthographic,
];

/// Every settings tab, and the tabs that share the column with them.
const SETTINGS_TABS: [panes::Tab; 8] = [
    panes::Tab::Tuning,
    panes::Tab::Nodes,
    panes::Tab::Scene,
    panes::Tab::Analyzer,
    panes::Tab::Video,
    panes::Tab::Panel,
    panes::Tab::Console,
    panes::Tab::Notes,
];

/// One settings pane whose content box is `width` points wide, as the shapes it
/// emitted. Driven through [`panes::Viewer`] rather than the dock, so a sweep
/// over widths costs one pane each instead of a whole window, and the width
/// under test is the pane's own rather than a window size minus chrome.
///
/// The dock's nesting IS reproduced, though, because the one thing it does that
/// a bare `Ui` does not is the thing these tests are about: egui_dock clips the
/// tab body to the whole body rect and only THEN insets it by
/// `tab_body.inner_margin` via a `Frame`, which does not clip. So a pane's clip
/// rect sits a margin's width OUTSIDE its content box, and a harness without
/// the margin cannot tell a control clamped to the content box from one clamped
/// to the painted edge — they are the same number there.
///
/// Tall on purpose (a pane's controls are a column, and the point here is the
/// other axis) and with the take controls switched on, so the Video tab draws
/// the record button and the Options field a real session has.
fn settings_pane_at_width(
    tab: panes::Tab,
    width: f32,
    projection: harmonigraph_scene::Projection,
) -> Vec<egui::epaint::ClippedShape> {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.take_supported = true;
    state.last_take_ready = true;
    state.camera.projection = projection;
    // A saved angle, so the Angle row has the button a real session gives it.
    state.camera_presets.push(CameraPreset { name: "Front".into(), yaw: 0.0, pitch: 0.0 });
    let backend = RecordingBackend::default();
    let ctx = egui::Context::default();
    crate::theme::apply_theme(&ctx);
    let margin = crate::theme::PANE_INNER_MARGIN;
    let body = egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(width + 2.0 * margin, 2400.0),
    );
    let out = ctx.run_ui(
        egui::RawInput { screen_rect: Some(body), time: Some(0.0), ..Default::default() },
        |ui| {
            // The body ui's clip is the whole body (the screen here); the pane
            // ui inside it is inset, exactly as the dock's Frame leaves it.
            let mut pane =
                ui.new_child(egui::UiBuilder::new().max_rect(body.shrink(margin)));
            let mut tab = tab;
            let mut viewer = panes::Viewer { state: &mut state, params: &backend, now: 0.0 };
            egui_dock::TabViewer::ui(&mut viewer, &mut pane, &mut tab);
        },
    );
    out.shapes
}

/// Where a pane's content box ends, in the coordinates
/// [`settings_pane_at_width`] lays it out at.
fn pane_content_right(width: f32) -> f32 {
    crate::theme::PANE_INNER_MARGIN + width
}

/// The projections worth drawing `tab` at: all of them for Tuning, whose
/// content depends on it (see [`PROJECTIONS`]), and the default alone for the
/// panes that draw the same thing either way.
fn projections_for(tab: panes::Tab) -> &'static [harmonigraph_scene::Projection] {
    if tab == panes::Tab::Tuning { &PROJECTIONS } else { &PROJECTIONS[..1] }
}

/// The y a named text run was painted at in `shapes`, or `None`.
fn text_y(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<f32> {
    shapes.iter().find_map(|cs| match &cs.shape {
        egui::Shape::Text(t) if t.galley.text() == needle => Some(t.pos.y),
        _ => None,
    })
}

/// The Options row is a label and its field on ONE line, at the column widths
/// wide enough to hold both.
///
/// Inside a wrapping row `Ui::available_width()` is the whole ROW, not what is
/// left of the current line: `Layout::available_size` takes its `main_wrap`
/// branch and returns `max_rect.width()`, ignoring the cursor. A field sized
/// from it therefore asks for the entire column, cannot fit after its own
/// label, and drops to a line of its own at every width — costing a row of
/// height in the pane whose height budget is the tight one (see
/// [`the_settings_column_needs_no_scroll_bar_at_the_window_it_was_dialled_in`]).
/// `available_size_before_wrap` is the accessor that means the rest of this
/// line, and the one a widget filling a row wants.
///
/// Swept only down to 240pt: below about 148 the label and a usable field
/// genuinely do not fit on one line, and wrapping is then the right answer.
#[test]
fn the_options_field_sits_beside_its_label() {
    for width in [423.0f32, 300.0, 240.0] {
        let shapes = settings_pane_at_width(panes::Tab::Video, width, PROJECTIONS[0]);
        let label = text_y(&shapes, "Options").expect("the Options label");
        // The field is not empty by default, so its own text locates it.
        let field = text_y(&shapes, "--size 1920x1080").expect("the Options field's text");
        // A field beside its label sits a few points off it (the text box has
        // its own margin); a field that has wrapped is a whole row away.
        assert!(
            (label - field).abs() < 10.0,
            "at {width}pt the Options label sits at y {label} and its field at y {field}: \
             the field has dropped onto a line of its own"
        );
    }
}

/// The bar tracks a pane drew, by width. A `ValueBar`/`RangeBar` track is the
/// one thing in a settings pane painted as a `BAR_HEIGHT`-tall rect in
/// `well()`: the accent fill over it is the same height in a different color,
/// and the record button's own `well()` panel is taller.
fn bar_track_widths(shapes: &[egui::epaint::ClippedShape]) -> Vec<f32> {
    let well = crate::theme::well();
    shapes
        .iter()
        .filter_map(|cs| match &cs.shape {
            egui::Shape::Rect(r) if r.fill == well && (r.rect.height() - 20.0).abs() < 0.6 => {
                Some(r.rect.width())
            }
            _ => None,
        })
        .collect()
}

/// Every bar in a settings pane is the same length, and that length is the
/// column's — so dragging the column narrower narrows all of them together.
///
/// What breaks it is invisible in the code that draws a bar, which is why this
/// is pinned rather than left to reading: egui's `Region::expand_to_include_rect`
/// unions `max_rect` as well as `min_rect`, so any control that overruns the
/// column widens the column for everything BELOW it, and a bar sizing itself
/// from bare `available_width` inherits the overrun as a floor it cannot shrink
/// past. Each bar's minimum length is then the width of the widest thing above
/// it — five different minimums down one pane, the bars under a wide row running
/// their value readout off the pane edge while the bars above it compress
/// properly. `widgets::bar_width` is the answer, and the reason it measures the
/// clip rect rather than trusting the layout.
///
/// Swept past the width where the pane's other controls stop fitting on purpose.
/// Above about 100pt nothing overruns at all (see below), so those widths would
/// pass whether a bar clamped itself or not; 100 and 80, where the record button
/// and the Options field have nowhere left to go, are where the clamp is the
/// only thing holding the bars level.
#[test]
fn every_bar_in_a_settings_pane_is_the_width_of_the_pane() {
    for width in [400.0f32, 240.0, 160.0, 120.0, 100.0, 80.0] {
        for tab in SETTINGS_TABS {
            for &projection in projections_for(tab) {
                let widths = bar_track_widths(&settings_pane_at_width(tab, width, projection));
                for bar in &widths {
                    assert!(
                        (bar - width).abs() < 1.0,
                        "{tab:?}/{projection:?} at {width}pt drew a {bar}pt bar \
                         (all of {widths:?})"
                    );
                }
            }
        }
    }
    // The sniffing above finds nothing if the bars stop being painted this way,
    // and a test that measures nothing passes. The Tuning pane is the deepest
    // stack of bars in the dock.
    let bars =
        bar_track_widths(&settings_pane_at_width(panes::Tab::Tuning, 400.0, PROJECTIONS[0])).len();
    assert!(bars >= 10, "only found {bars} bar tracks in the Tuning pane; has the paint changed?");
}

/// No settings pane's controls run out past the column, at any width worth
/// dragging one to. Off the pane edge a control cannot be read, clicked, or
/// dragged to its end, and horizontal scrolling is deliberately off in the dock
/// (see `panes::Viewer::scroll_bars`), so there is no way to reach it.
///
/// Three things hold it: rows wrap, and so do the labels of the buttons in them
/// (`widgets::button_row`); bars take the column's visible width
/// (`widgets::bar_width`); and a bar's name elides against its own value readout
/// instead of running over it and out of the pane.
///
/// 120pt is the narrowest pinned because it is the last width where everything
/// still fits. Below about 100 what is left is widgets that wrap nothing and
/// have nowhere to wrap to — the record button, a `toggle_switch` label, the
/// Options field — and the answer there would be to elide those too, which costs
/// every reader something to buy back a column nobody drags to.
///
/// The column opens at around 423pt (`state::SETTINGS_SPLIT` of the reference
/// window) and fits there, which is why this went unnoticed: the overrun starts
/// somewhere under 400, and by 300 the Tuning pane was running 32pt of bar off
/// its own edge. It is a resize bug, so the sweep is the test.
#[test]
fn no_settings_pane_overruns_a_narrow_column() {
    for width in [400.0f32, 300.0, 240.0, 200.0, 160.0, 120.0] {
        let edge = pane_content_right(width);
        // The pane's own clip is the tab body, a margin wider than the content
        // box on each side.
        let body_right = edge + crate::theme::PANE_INNER_MARGIN;
        let panes = SETTINGS_TABS
            .into_iter()
            .flat_map(|tab| projections_for(tab).iter().map(move |&p| (tab, p)));
        for (tab, projection) in panes {
            let shapes = settings_pane_at_width(tab, width, projection);
            let over_edge = |cs: &egui::epaint::ClippedShape| {
                let rect = cs.shape.visual_bounding_rect();
                // Shapes that carry no geometry answer with an inverted or
                // infinite rect; egui's own `is_finite` lets those through.
                if !rect.is_finite() || rect.width() > 1.0e4 {
                    return None;
                }
                // A widget that set its own clip, tighter than the body, is
                // managing its own overflow — a single-line text box scrolls
                // its content inside the field, so its galley is routinely
                // wider than the box and correctly cut off there. Only the
                // body's own clip means "cut off by the pane", which is the
                // thing that can be neither reached nor read.
                if cs.clip_rect.right() < body_right - 0.5 {
                    return None;
                }
                (rect.right() - edge > 1.0).then(|| rect.right() - edge)
            };
            let worst = shapes
                .iter()
                .filter_map(|cs| over_edge(cs).map(|over| (over, cs)))
                .max_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(over, cs)| {
                    let what = match &cs.shape {
                        egui::Shape::Text(t) => format!("{:?}", t.galley.text()),
                        other => format!("{other:?}").chars().take(40).collect(),
                    };
                    (over, what)
                });
            assert!(
                worst.is_none(),
                "{tab:?}/{projection:?} at {width}pt ran {:?} past the pane edge",
                worst.unwrap()
            );
        }
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
    unfold_the_readout_panes(&mut state);
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

/// The performance overlay hangs off the analyzer pane; off the lattice when
/// that pane isn't on screen; off the editor, clear of the tab bar, when
/// neither is. All three land somewhere no tab bar's collapse arrow is.
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
    let hud = hud_of(&output);
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
    // screen; the overlay then falls back to the OTHER picture pane.
    let path = state.dock.find_tab(&panes::Tab::Spectral).expect("Spectral is docked");
    let egui_dock::Node::Leaf(leaf) = &mut state.dock[path.surface][path.node] else {
        panic!("Spectral should live in a leaf");
    };
    leaf.collapsed = true;
    frame(&mut state);
    let output = frame(&mut state);
    let lattice = state.dock.find_tab(&panes::Tab::Lattice).expect("Lattice is docked");
    let egui_dock::Node::Leaf(lattice) = &state.dock[lattice.surface][lattice.node] else {
        panic!("Lattice should live in a leaf");
    };
    assert_eq!(
        perf_overlay_area(&state, screen),
        lattice.viewport,
        "a collapsed analyzer should hand the overlay to the lattice, not to the window",
    );

    // ...and the point of that, which is what the fallback is FOR: the HUD is
    // painted on a foreground layer over the whole dock, so hanging it off the
    // window puts it on the chrome along the top — the settings column's tab
    // bar, and the collapse arrow at the left of every bar, which is the
    // control that brings a folded pane back. A tab body starts below its own
    // bar, so landing in one is what keeps it clear of all of them.
    fn hud_of(output: &egui::FullOutput) -> egui::Rect {
        let plate = egui::Color32::from_black_alpha(0xC0);
        output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) if rect.fill == plate => Some(rect.rect),
                _ => None,
            })
            .expect("the overlay should paint its backing plate")
    }
    fn clear_of_every_tab_bar(state: &SharedState, hud: egui::Rect, what: &str) {
        for node in state.dock.main_surface().iter() {
            let egui_dock::Node::Leaf(leaf) = node else {
                continue;
            };
            let mut bar = leaf.rect;
            bar.max.y = bar.min.y + crate::theme::TAB_BAR_HEIGHT;
            assert!(
                !hud.intersects(bar),
                "{what}: the HUD covers a tab bar and its collapse arrow: {hud:?} over {bar:?}",
            );
        }
    }
    clear_of_every_tab_bar(&state, hud_of(&output), "on the lattice");

    // Fold the lattice too and there is no picture left to hang off, which is
    // the last resort. It is the only branch that does arithmetic — the editor
    // rect pushed down past the tab bar — and the arithmetic is the whole of
    // what keeps the HUD off the collapse arrows in the one state where those
    // arrows are the only way back.
    let path = state.dock.find_tab(&panes::Tab::Lattice).expect("Lattice is docked");
    let egui_dock::Node::Leaf(leaf) = &mut state.dock[path.surface][path.node] else {
        panic!("Lattice should live in a leaf");
    };
    leaf.collapsed = true;
    frame(&mut state);
    let output = frame(&mut state);
    let area = perf_overlay_area(&state, screen);
    assert_eq!(
        area.min.y,
        screen.min.y + crate::theme::TAB_BAR_HEIGHT,
        "with neither picture on screen the overlay should clear the tab bar: {area:?}",
    );
    clear_of_every_tab_bar(&state, hud_of(&output), "with both pictures folded");
}

/// Landing in the analyzer's body is only half of staying out of the way: the
/// HUD is painted on a foreground layer whose clip is the whole screen, so a
/// pane too narrow to hold it does not crop it — it spills across the separator
/// and over the settings column, and over whatever collapse arrow is there.
///
/// The analyzer is the NARROWEST pane in `default_dock` (0.2016 of the window),
/// so it is the first to run out of room, and a sideways fold can drive the
/// window to its floor without the user ever dragging it there.
#[test]
fn the_perf_overlay_stays_inside_its_pane_at_the_narrowest_window() {
    // The plugin's own minimum editor width (`MIN_SIZE` in the plugin crate's
    // editor), which is a window the shell will actually hand the UI.
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 800.0));
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let backend = RecordingBackend::default();
    let ctx = egui::Context::default();
    let mut t = 0.0;
    let mut frame = |state: &mut SharedState| {
        t += 1.0 / 60.0;
        let raw =
            egui::RawInput { screen_rect: Some(screen), time: Some(t), ..Default::default() };
        ctx.run_ui(raw, |ui| root_ui(ui, state, &backend, t))
    };
    frame(&mut state);
    let output = frame(&mut state);

    let plate = egui::Color32::from_black_alpha(0xC0);
    let hud = output
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) if rect.fill == plate => Some(rect.rect),
            _ => None,
        })
        .expect("the overlay should paint its backing plate");
    let area = perf_overlay_area(&state, screen);
    assert!(
        area.contains_rect(hud),
        "the HUD ran {:.0}pt past its {:.0}pt pane: {hud:?} in {area:?}",
        hud.right() - area.right(),
        area.width(),
    );
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

/// The lattice's label-size bar and the clamp its value is persisted through
/// offer the same range.
///
/// `SCALE_BAR_RANGE` says it is "one range for the three of them", and for two
/// of them it is the constant itself: `sane_scale` fits `marking_scale` and
/// `note_name_scale` to it on load, so those two cannot drift and are not
/// worth asserting — an assertion that clamping to a constant lands inside it
/// only restates `clamp`. The lattice's `label_scale` is the one that can:
/// `ViewConfig` lives in `harmonigraph-scene`, which is BELOW this crate, so the
/// range is not visible there and `migrate_legacy` clamps to a written-out
/// copy of the same two numbers.
///
/// Nothing ties the copy to the original. Widen the bar and a saved view keeps
/// loading at the old ceiling, which is a setting that will not stay where it
/// is put — silently, and only for the one of the three that is a different
/// crate. This is what notices.
#[test]
fn the_lattice_label_bar_persists_through_the_range_it_offers() {
    let through_view = |scale: f32| {
        let mut view = harmonigraph_scene::ViewConfig { label_scale: scale, ..Default::default() };
        view.migrate_legacy();
        view.label_scale
    };
    let (low, high) = (*SCALE_BAR_RANGE.start(), *SCALE_BAR_RANGE.end());
    assert_eq!(through_view(low - 1.0), low, "the bar's floor");
    assert_eq!(through_view(high + 1.0), high, "...and its ceiling");
}

/// Draw enough distinct marks in ONE pass to outrun the cache limit, so
/// eviction is running while the pass is still drawing. Returns the texture
/// ids the pass drew, and the ids it asked egui to destroy.
fn mark_cache_pass(ctx: &egui::Context) -> (std::collections::HashSet<egui::TextureId>, Vec<egui::TextureId>) {
    use panes::lattice::{MARK_CACHE_LIMIT, MARK_SIZE};
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
    // Both comma marks, so each size contributes more than one key.
    let name =
        harmonigraph_core::NoteName { letter: 'C', sharps: 1, syntonic_commas: 1, septimal_commas: 1 };
    let out = ctx.run_ui(
        egui::RawInput { screen_rect: Some(screen), ..Default::default() },
        |ui| {
            let mut batch = crate::text::TextBatch::default();
            // One mark per whole pixel size, far enough past the limit that
            // the pass evicts marks it has already painted.
            for size_px in 3..=(3 + MARK_CACHE_LIMIT / 2 + 8) {
                panes::lattice::draw_stacked_name(
                    &mut batch,
                    ui.painter(),
                    egui::pos2(200.0, 200.0),
                    name,
                    egui::Color32::WHITE,
                    egui::Color32::BLACK,
                    size_px as f32 / MARK_SIZE,
                );
            }
        },
    );
    let drawn = out
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Mesh(mesh) => Some(mesh.texture_id),
            _ => None,
        })
        .collect();
    (drawn, out.textures_delta.free)
}

/// A pass that fills the mark cache must not destroy a texture it has
/// already drawn.
///
/// Eviction drops the last handle to a bitmap, which makes egui queue that
/// id into `textures_delta.free` -- and egui-baseview's wgpu renderer
/// applies those frees BEFORE it submits the encoder. So a mark evicted
/// midway through a pass is destroyed while the draw commands naming it are
/// still queued, and `Queue::submit` fails validation with "Texture ... has
/// been destroyed", which wgpu treats as fatal. The victim is arbitrary, so
/// it is sometimes a mark this pass has already painted.
///
/// Reachable from any control that walks a mark's key: the sizes zooming
/// steps through, or the weight when that was still a setting. Each pass
/// mints fresh keys, so the cache sits at its limit and evicts on every
/// insert.
#[test]
fn filling_the_mark_cache_never_frees_a_texture_the_pass_drew() {
    let ctx = egui::Context::default();
    theme::apply_theme(&ctx);
    let (drawn, freed) = mark_cache_pass(&ctx);
    let bad: Vec<_> = freed.iter().copied().filter(|id| drawn.contains(id)).collect();
    assert!(bad.is_empty(), "the pass freed {} textures it had drawn: {bad:?}", bad.len());
}

/// ...and the pass AFTER it must actually destroy them.
///
/// The retention is a delay, not a reprieve: holding an evicted bitmap
/// forever would trade the crash above for a leak that grows for as long as
/// the editor is open, since a zoom drag mints fresh keys every pass. This
/// pins the half the single-pass test cannot see -- with the prune deleted
/// that test still passes, because pass 0 frees nothing either way.
#[test]
fn the_next_pass_destroys_what_the_last_one_retired() {
    let ctx = egui::Context::default();
    theme::apply_theme(&ctx);
    let (_, freed_first) = mark_cache_pass(&ctx);
    assert!(freed_first.is_empty(), "the first pass should hold its evictions, freed {freed_first:?}");

    let (drawn, freed_second) = mark_cache_pass(&ctx);
    assert!(
        !freed_second.is_empty(),
        "the second pass should destroy what the first retired, or they accumulate"
    );
    let bad: Vec<_> = freed_second.iter().copied().filter(|id| drawn.contains(id)).collect();
    assert!(bad.is_empty(), "the second pass freed {} textures it had drawn: {bad:?}", bad.len());
}



/// The pane names painted up a rail this frame, top to bottom.
///
/// `fold::paint` sets them a quarter turn anticlockwise, which nothing else in
/// the dock does — so the angle is what tells a rail's name from a tab title,
/// and the rail's own rectangle is what keeps another pane's rotated text out.
///
/// Each comes with the y its text STARTS at — the top of the drawn word. A
/// name is anchored at its far end and drawn upwards, so that is the anchor
/// less the galley's length.
fn rail_labels(output: &egui::FullOutput, rail: egui::Rect) -> Vec<(String, f32)> {
    fn walk(shape: &egui::Shape, rail: egui::Rect, found: &mut Vec<(f32, String)>) {
        match shape {
            egui::Shape::Text(text) if text.angle != 0.0 && rail.contains(text.pos) => {
                found.push((text.pos.y - text.galley.size().x, text.galley.text().to_owned()));
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, rail, found)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &output.shapes {
        walk(&clipped.shape, rail, &mut found);
    }
    found.sort_by(|(a, _), (b, _)| a.total_cmp(b));
    found.into_iter().map(|(top, name)| (name, top)).collect()
}

/// The rail a folded subtree left behind: the leaves' rectangles, which are
/// the rail's own once they have nothing else in them.
fn rail_rect(state: &SharedState, tabs: &[panes::Tab]) -> egui::Rect {
    tabs.iter().fold(egui::Rect::NOTHING, |rail, tab| {
        let path = state.dock.find_tab(tab).expect("tab is in the dock");
        rail.union(state.dock[path.surface][path.node].rect().expect("the leaf is laid out"))
    })
}

/// A folded column names every pane in it, not just the one at the bottom.
///
/// egui_dock stacks a collapsed leaf's tab bar at the top of the column and
/// hands everything below it to whichever leaf is LAST, so the rail carried a
/// single name — "Notes", the pane at the bottom of the settings column and
/// the one that says least about what folded. The names now divide the rail
/// by the fractions the column is dialled at, so both of them are on it.
#[test]
fn a_folded_column_names_every_pane_in_it() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.min_window_width = 400.0;
    let mut h = DockHarness::new();
    h.settle(&mut state);
    // Notes/Console is folded in the default layout, so collapsing the
    // settings leaf collapses the column itself and the whole of it folds
    // sideways to one rail.
    let output = h.collapse_click(&mut state, panes::Tab::Tuning);
    let rail = rail_rect(&state, &[panes::Tab::Tuning, panes::Tab::Notes]);
    assert!(rail.width() < 40.0, "the column should have folded to a rail ({rail:?})");
    let labels = rail_labels(&output, rail);
    assert_eq!(
        labels.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(),
        ["Tuning", "Notes"],
        "both panes in the folded column should be named, in the order they are stacked",
    );
    // Each in ITS share of the rail, under its own arrow: the shares are the
    // column's own fractions (the log leaf is dialled at 0.45 of it), and a
    // name hangs one tab bar plus a little padding below the top of its share,
    // which is the arrow that brings that pane back.
    let boundary = rail.top() + rail.height() * 0.55;
    let under = |top: f32| top..top + crate::theme::TAB_BAR_HEIGHT + 20.0;
    assert!(
        under(rail.top()).contains(&labels[0].1),
        "the upper pane's name should sit under the arrow at the top of the rail: {labels:?}",
    );
    assert!(
        under(boundary).contains(&labels[1].1),
        "the lower pane's name should sit under its own arrow at {boundary}: {labels:?}",
    );
}

/// Two panes folded side by side name themselves under their own arrows.
///
/// A folded pair is two rails, and their collapse arrows end up next to each
/// other at the top of the window pointing the SAME way — both panes come back
/// into the space the pair gave up, so the direction cannot separate them.
/// That leaves the name, and a name floating at the middle of its rail is most
/// of a window away from the arrow it belongs to. Under the arrow the two read
/// as a caption each.
#[test]
fn a_folded_pair_names_each_rail_under_its_own_arrow() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.min_window_width = 400.0;
    let mut h = DockHarness::new();
    h.settle(&mut state);
    // The picture pair: collapsing both leaves the split itself collapsed, so
    // the whole subtree folds as one and comes out two rails wide.
    h.collapse_click(&mut state, panes::Tab::Lattice);
    let output = h.collapse_click(&mut state, panes::Tab::Spectral);

    for tab in [panes::Tab::Lattice, panes::Tab::Spectral] {
        let rail = rail_rect(&state, &[tab]);
        assert!(rail.width() < 40.0, "{tab:?} should have folded to a rail ({rail:?})");
        let labels = rail_labels(&output, rail);
        let (name, top) = labels.first().expect("a rail says which pane it is");
        assert_eq!(name, crate::panes::tab_title(&tab), "each rail names its own pane");
        // The arrow is the leaf's tab bar, at the top of the rail. Clear of
        // it, and within a word's length of it — not adrift at mid-window,
        // which for this dock would be past 380.
        let arrow = rail.top() + crate::theme::TAB_BAR_HEIGHT;
        assert!(
            (arrow..arrow + 20.0).contains(top),
            "{name} sits at {top}, and its arrow ends at {arrow}",
        );
    }
}

/// Whether the leaf holding `tab` is folded away.
fn collapsed(state: &SharedState, tab: panes::Tab) -> bool {
    let path = state.dock.find_tab(&tab).expect("tab is in the dock");
    state.dock[path.surface][path.node].is_collapsed()
}

/// A click on the arrow at the top of a folded column's LOWER share opens the
/// pane that share belongs to.
///
/// egui_dock puts a collapsed leaf's arrow at the top of its own rectangle,
/// and down a folded column those rectangles start one after another — so its
/// arrows stack in the first inches of the rail, out of reach of the panes
/// they name. `fold` places them on the shares instead and takes the clicks
/// itself, which is the half of this that cannot be done in paint alone.
#[test]
fn a_folded_columns_lower_arrow_opens_its_own_pane() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.min_window_width = 400.0;
    let mut h = DockHarness::new();
    h.settle(&mut state);
    h.collapse_click(&mut state, panes::Tab::Tuning);
    let rail = rail_rect(&state, &[panes::Tab::Tuning, panes::Tab::Notes]);
    assert!(collapsed(&state, panes::Tab::Tuning) && collapsed(&state, panes::Tab::Notes));

    // The lower share's own arrow, at 0.55 down the rail.
    let at = egui::pos2(
        rail.left() + 12.0,
        rail.top() + rail.height() * 0.55 + crate::theme::TAB_BAR_HEIGHT * 0.5,
    );
    h.frame(&mut state, vec![egui::Event::PointerMoved(at)]);
    h.frame(&mut state, vec![egui::Event::PointerMoved(at), press(at, true)]);
    h.frame(&mut state, vec![press(at, false)]);
    h.settle_folds(&mut state);

    assert!(!collapsed(&state, panes::Tab::Notes), "the lower arrow opens the lower pane");
    assert!(
        collapsed(&state, panes::Tab::Tuning),
        "and only that one — the settings leaf keeps its own arrow",
    );
    // The column is a column again, and the leaf still folded is one tab bar
    // tall. That last one is where a botched collapsed-leaf count shows up:
    // not as a pane that fails to open, but as a bar drawn some multiple of a
    // tab bar thick afterwards (see `fold::uncollapse`).
    let path = state.dock.find_tab(&panes::Tab::Tuning).expect("tab is in the dock");
    let folded = state.dock[path.surface][path.node].rect().expect("laid out");
    assert!(folded.width() > 100.0, "the rail should be a column again: {folded:?}");
    assert!(
        (folded.height() - crate::theme::TAB_BAR_HEIGHT).abs() < 4.0,
        "the settings leaf should be one tab bar tall, not {}",
        folded.height(),
    );
}

/// The arrow egui_dock left stacked at the top of the rail no longer opens
/// anything: it sits inside the share above it now, under another pane's name,
/// and a click there would open a pane the rail says nothing about.
#[test]
fn a_folded_columns_stacked_arrow_is_inert() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.min_window_width = 400.0;
    let mut h = DockHarness::new();
    h.settle(&mut state);
    h.collapse_click(&mut state, panes::Tab::Tuning);
    let rail = rail_rect(&state, &[panes::Tab::Tuning, panes::Tab::Notes]);

    // Where egui_dock puts the log leaf's button: directly under the settings
    // leaf's, one tab bar down from the top of the rail.
    let path = state.dock.find_tab(&panes::Tab::Notes).expect("tab is in the dock");
    let stacked = state.dock[path.surface][path.node].rect().expect("laid out");
    let at = stacked.left_top() + egui::vec2(12.0, crate::theme::TAB_BAR_HEIGHT * 0.5);
    assert!(
        at.y < rail.top() + rail.height() * 0.5,
        "the stacked arrow should be near the top of the rail, at {at:?}",
    );
    h.frame(&mut state, vec![egui::Event::PointerMoved(at)]);
    h.frame(&mut state, vec![egui::Event::PointerMoved(at), press(at, true)]);
    h.frame(&mut state, vec![press(at, false)]);
    h.settle_folds(&mut state);

    assert!(
        collapsed(&state, panes::Tab::Notes),
        "the log pane should not open from a button that is no longer drawn",
    );
}
