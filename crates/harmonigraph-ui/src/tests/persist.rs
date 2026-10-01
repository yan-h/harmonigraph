//! What survives a session: round-trips through [`crate::state::UiPersist`], the version
//! floor under it, and what a blob costs when a key it carries — or one it is
//! missing — is not the shape this build expects.

use super::harness::collapsed;
use super::probe::fresh;
use crate::state::UI_PERSIST_VERSION;
use crate::*;
use harmonigraph_scene::{Camera, NoteNames};

fn set_console_collapsed(state: &mut SharedState, collapsed: bool) {
    state.workspace.layout.select(if collapsed { panes::Tab::Tuning } else { panes::Tab::Console });
    state.workspace.layout.folded[workspace::Section::Settings as usize] = false;
}

/// A state dialled away from fresh wherever a retirement below sits, so a
/// blob that survived is distinguishable from one that sank and reverted to
/// defaults, and so every live variant the dropped ones are spliced over is in
/// the blob to splice over.
fn dialled() -> SharedState {
    let mut state = fresh();
    let appearance = &mut state.picture.appearance;
    appearance.camera.yaw = 1.23;
    appearance.camera.distance = 18.0;
    appearance.camera.cabinet_scale = 0.7;
    appearance.view.label_scale = 0.7;
    appearance.view.ring_gap = 0.02;
    appearance.view.max_sevens = 3;
    appearance.view.note_animation.order = harmonigraph_scene::AnimationOrder::Circular;
    appearance.view.note_animation.stagger_spread = 0.63;
    appearance.view.atmosphere.texture = harmonigraph_scene::LatticeTexture::None;
    appearance.spectrum.orientation = crate::SpectralOrientation::Left;
    appearance.spectrum.roll_thickness = 1.75;
    appearance.spectrum.low_midi = 40.5;
    appearance.render.short_edge = 2160;
    appearance.render.frame.split = 0.37;
    appearance.render.spectrogram = crate::SpectrogramRender::Scrolling;
    state.workspace.interaction.ui_scale = 1.25;
    state
}

/// Where `key:` opens a pair in `blob` — after a `(` or a `,`, so a key that
/// merely ends in the same letters (`spectrogram:` inside `x_spectrogram:`) is
/// not taken for it.
fn pair_starts(blob: &str, key: &str) -> Vec<usize> {
    blob.match_indices(&format!("{key}:"))
        .filter(|(i, _)| *i > 0 && matches!(blob.as_bytes()[i - 1], b'(' | b','))
        .map(|(i, _)| i)
        .collect()
}

/// The `(…)` struct whose pair opens at `at`, string-aware so a quoted paren
/// cannot unbalance it.
fn enclosing_struct(blob: &str, at: usize) -> &str {
    let (mut opens, mut in_string, mut escaped) = (Vec::new(), false, false);
    let mut target = None;
    for (i, c) in blob.char_indices() {
        if i == at {
            target = Some(opens.len());
        }
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '(' | '[' => opens.push(i),
            ')' | ']' => {
                let open = opens.pop().expect("balanced blob");
                if target == Some(opens.len() + 1) {
                    return &blob[open..=i];
                }
            }
            _ => {}
        }
    }
    panic!("no struct encloses byte {at}");
}

/// Every key a retirement has left in saved projects, as `(a live key in the
/// same struct, the retired pairs as those builds wrote them)`: the retired
/// pairs are spliced back in ahead of the live key.
///
/// The VALUE shapes are the point as much as the names. A number is skipped by
/// any parser; a bare identifier (`Fibres`, `Pop`) is an enum token with no
/// type left to parse it into, and a string and `NaN` are tokens of their own.
/// Retiring a FIELD is safe for every one of them, where retiring a VARIANT is
/// not — that is [`DROPPED_VARIANTS`].
///
/// Retiring another key is a row here, not another test. Only keys a blob at
/// or above the version floor can hold earn one: anything retired before it
/// is refused with the blob that carries it.
const RETIRED_KEYS: &[(&str, &str)] = &[
    // The cloud's sampling grid.
    ("cloud_depth", "cloud_tile:20.0,cloud_pixel:4.0,"),
    // Node spacing, including a non-finite one.
    ("label_scale", "spacing:3.0,"),
    ("label_scale", "spacing:NaN,"),
    // The independent angular gap. `ring_gap` is the shared one now, and the
    // dialled 0.02 must not be overwritten by it.
    ("ring_gap", "octave_gap:0.17,"),
    // The entire combined lattice material key is retired, including its old variants.
    ("material_style", "material:Fibres,"),
    ("material_style", "material:Liquid,"),
    // The hidden renderer override, a string.
    ("short_edge", "renderer_path:\"/old/renderer\","),
    // The view's retired switches.
    ("label_scale", "show_labels:false,sounding_ink:12.0,mark_melody:false,mark_bass:false,"),
    // The note animation's own retired fields, inside its struct.
    ("stagger_spread", "animation:Pop,start_size:0.2,"),
    // The overlay switches, moved out of the view to the editor's top level:
    // one inside a view must not reach the top-level key of the same name.
    ("render_scale", "show_perf:true,show_perf_detail:true,"),
    // The Spiral's framing, moved out of the appearance to the editor's top
    // level for the same reason, and a struct where those were flags.
    ("spectrum", "spiral:(zoom:3.0,look:(0.4,-0.6)),"),
];

/// A retired key costs nothing: through both doors — the editor's
/// `load_persist`, and the `AppearanceDocument::parse` that take replay and
/// explicit export share — the rest of the document loads at the values it
/// holds, and the key is gone on the next save.
///
/// `load_persist` takes the whole `UiPersist` or nothing, so a key that failed
/// to parse would not degrade: it would discard the dock, the camera and the
/// view with it. Every retirement rides on serde ignoring what it has no field
/// for, and this is where that is held.
#[test]
fn a_retired_key_is_ignored_and_the_rest_survives() {
    let state = dialled();
    let saved = state.save_persist();
    let appearance = state.picture.appearance.serialize();
    for &(anchor, retired) in RETIRED_KEYS {
        let splice = |blob: &str| {
            let at = pair_starts(blob, anchor);
            assert_eq!(at.len(), 1, "{anchor:?} must open exactly one pair in {blob}");
            format!("{}{retired}{}", &blob[..at[0]], &blob[at[0]..])
        };
        // Retired from the anchor's struct rather than merely renamed: this
        // build writes none of them THERE. Checked in that struct alone, since
        // a key can move elsewhere under the same name (`show_perf`).
        let at = pair_starts(&saved, anchor);
        assert_eq!(at.len(), 1, "{anchor:?} must open exactly one pair in {saved}");
        let live = top_level_pairs(enclosing_struct(&saved, at[0]));
        for (key, _) in top_level_pairs(&format!("({})", retired.trim_end_matches(','))) {
            assert!(live.iter().all(|(k, _)| *k != key), "{key:?} is live, not retired");
        }

        let mut restored = fresh();
        assert!(restored.load_persist(&splice(&saved)), "{retired} sank the editor blob");
        assert_eq!(restored.save_persist(), saved, "{retired} changed what the editor loaded");

        let parsed = AppearanceDocument::parse(&splice(&appearance))
            .unwrap_or_else(|e| panic!("{retired} sank the appearance: {e}"));
        assert_eq!(parsed.serialize(), appearance, "{retired} changed what the appearance loaded");
    }
}

/// Every variant a retirement has dropped from a persisted enum, as `(key, a
/// live variant, the dropped one, whether the key is part of the appearance)`.
/// The last is `false` for editor-only state, which only the editor door reads.
///
/// Retiring another variant is a row here, not another test.
const DROPPED_VARIANTS: &[(&str, &str, &str, bool)] = &[
    ("orientation", "Left", "Diagonal", true),
    ("order", "Circular", "OddEvenStagger", true),
    ("spectrogram", "Scrolling", "Playhead", true),
    // The selected settings tab lives in the layout, so removing a tab is a
    // variant break too.
    ("settings_tab", "Tuning", "Display", false),
    // The Mappings tab's variant, before it took the tab's title.
    ("settings_tab", "Tuning", "Colors", false),
];

/// A dropped variant refuses the WHOLE document, and says so on the Console.
///
/// serde cannot build a partial `UiPersist` around a variant it has no arm for,
/// and the parse fails BEFORE the version is read, so the floor cannot catch it
/// however high it is set. That is a break this build accepts rather than
/// carries an alias for, and what makes it acceptable is that it is loud: the
/// Console opens on the reason, and nothing else from the blob is applied.
#[test]
fn a_dropped_variant_refuses_the_whole_document_and_says_so() {
    let state = dialled();
    let saved = state.save_persist();
    let appearance = state.picture.appearance.serialize();
    for &(key, live, dropped, in_appearance) in DROPPED_VARIANTS {
        let swap = |blob: &str| {
            let (was, now) = (format!("{key}:{live}"), format!("{key}:{dropped}"));
            let at: Vec<_> = pair_starts(blob, key)
                .into_iter()
                .filter(|&i| blob[i..].starts_with(&was))
                .collect();
            assert_eq!(at.len(), 1, "{was} must sit exactly once in {blob}");
            format!("{}{now}{}", &blob[..at[0]], &blob[at[0] + was.len()..])
        };

        let mut restored = fresh();
        let before = restored.save_persist();
        assert!(!restored.load_persist(&swap(&saved)), "{key}:{dropped} was applied");
        assert!(!collapsed(&restored, panes::Tab::Console), "{dropped}: the refusal is hidden");
        assert!(
            restored
                .picture
                .runtime
                .console
                .lines()
                .any(|line| line.contains("did not parse") && line.contains(dropped)),
            "{dropped}: the refusal does not name it; console holds {:?}",
            restored.picture.runtime.console.lines().collect::<Vec<_>>(),
        );
        set_console_collapsed(&mut restored, true);
        assert_eq!(restored.save_persist(), before, "{dropped}: more than the report was applied");

        assert_eq!(!pair_starts(&appearance, key).is_empty(), in_appearance, "{key}: which doors");
        if in_appearance {
            assert!(AppearanceDocument::parse(&swap(&appearance)).is_err(), "{dropped} parsed");
        }
    }
}

#[test]
fn persist_round_trips_camera_and_view() {
    let mut state = fresh();
    state.picture.appearance.camera.yaw = 1.23;
    state.picture.appearance.camera.distance = 18.0;
    state.picture.appearance.view.max_sevens = 3;
    // Non-default values throughout, so the fields prove they
    // round-trip rather than matching the defaults by luck. Inside their own
    // bars too: `sanitize` holds the Layers widths to the ranges their handles
    // offer, so a width past a ceiling would be testing the repair rather than
    // the round trip (the band's ceiling is `RING_WIDTH_MAX`, 0.6).
    state.picture.appearance.view.band_width = 0.5;
    state.picture.appearance.view.spectral_ring_width = 0.1;
    state.picture.appearance.view.ring_gap = 0.02;
    // A wheel that is neither the default count nor a center on a C, so the
    // pair proves it round-trips rather than landing back on something the
    // layout would have produced anyway. The center carries a fraction of a
    // semitone the bar cannot set, since the field is a continuous pitch and
    // a blob is entitled to one.
    state.picture.appearance.view.octave_count = 7;
    state.picture.appearance.view.octave_center = 64.5;
    state.picture.appearance.view.plus_arm = 0.5;
    state.picture.appearance.view.plus_taper = 0.07;
    for (index, group) in state.picture.appearance.view.shadow.groups_mut().into_iter().enumerate()
    {
        group.width = 0.1 + index as f32 * 0.1;
        group.depth = 0.2 + index as f32 * 0.1;
        // Off the fresh 1.0 and different per group, on the rule the head of
        // this test states: `groups()` compares whole styles, so a falloff left
        // at its default would round-trip by matching the default rather than
        // by being carried.
        group.falloff = 0.8 + index as f32 * 0.4;
        group.kernel = if index % 2 == 0 {
            harmonigraph_scene::ShadowKernel::Distance
        } else {
            harmonigraph_scene::ShadowKernel::Gaussian
        };
    }
    // Which nodes are named, and the fresh view keeps the past -- so either
    // other mode is a value a project has to keep, and the one a fresh view
    // would overwrite if it did not.
    state.picture.appearance.view.note_names = NoteNames::All;
    state.picture.appearance.view.meantone = true;
    // Off is the non-default here, and the one a project has to keep: the
    // detect would otherwise re-engage the mode the user switched it off for.
    state.picture.appearance.view.meantone_auto = false;
    // The septimal comma's pair of switches carries the same way, and set the
    // other way round from the syntonic one's so a blob that crossed them
    // could not pass.
    state.picture.appearance.view.marvel = false;
    state.picture.appearance.view.marvel_auto = true;
    state.workspace.interaction.camera_presets.push(CameraPreset {
        name: "reading".into(),
        yaw: 0.7,
        pitch: 0.2,
    });
    let saved = state.save_persist();

    let mut restored = fresh();
    restored.load_persist(&saved);
    assert_eq!(restored.picture.appearance.camera.yaw, 1.23);
    assert_eq!(restored.picture.appearance.camera.distance, 18.0);
    assert_eq!(restored.picture.appearance.view.max_sevens, 3);
    assert_eq!(restored.picture.appearance.view.band_width, 0.5);
    assert_eq!(restored.picture.appearance.view.spectral_ring_width, 0.1);
    assert_eq!(restored.picture.appearance.view.ring_gap, 0.02);
    assert_eq!(
        (
            restored.picture.appearance.view.octave_count,
            restored.picture.appearance.view.octave_center
        ),
        (7, 64.5)
    );
    assert_eq!(restored.picture.appearance.view.plus_arm, 0.5);
    assert_eq!(restored.picture.appearance.view.plus_taper, 0.07, "and the taper on their ends");
    assert_eq!(
        restored.picture.appearance.view.shadow.groups(),
        state.picture.appearance.view.shadow.groups(),
        "all four independent Shadow styles round-trip",
    );
    assert!(!saved.contains("spiral_shadow"), "the spiral must not grow a persisted style");
    assert_eq!(
        restored.picture.appearance.view.note_names,
        NoteNames::All,
        "a non-default note-name mode round-trips",
    );
    assert!(restored.picture.appearance.view.meantone);
    assert!(
        !restored.picture.appearance.view.meantone_auto,
        "a switched-off auto-detect round-trips"
    );
    assert!(!restored.picture.appearance.view.marvel, "each comma keeps its own mode");
    assert!(restored.picture.appearance.view.marvel_auto, "and its own detect");
    assert_eq!(restored.workspace.interaction.camera_presets.len(), 1);
    assert_eq!(restored.workspace.interaction.camera_presets[0].name, "reading");
    assert_eq!(restored.workspace.interaction.camera_presets[0].yaw, 0.7);
}

/// Loaded counts agree with the drawable range and the setting's readout.
#[test]
fn a_blob_naming_more_wheel_than_fits_opens_on_what_fits() {
    for (stored, expected) in [(0, 2), (1, 2), (99, 11)] {
        let mut state = fresh();
        state.picture.appearance.view.octave_count = stored;
        let mut restored = fresh();
        assert!(restored.load_persist(&state.save_persist()));
        assert_eq!(restored.picture.appearance.view.octave_count, expected);
    }
}

/// A hand-edited level range comes back drawable, which the pitch pair has
/// always been made to and this pair was not.
///
/// A NaN end is the one that has to be repaired rather than clamped: it loses
/// every comparison, so it survives a `max` and reaches `loudness` as the
/// divisor of a mapping that then paints the NaN geometry egui panics on —
/// inside the host, as a project opens. `loudness_raw`'s own guard answers an
/// inverted or collapsed pair and cannot answer this one.
///
/// Two controls write the pair now — the Level range bar and the drag across the
/// spectrum — and neither can produce either shape, which is what makes the
/// blob the only way in.
#[test]
fn a_blob_naming_an_undrawable_level_range_opens_on_a_drawable_one() {
    for (floor, ceiling, hint) in [
        ("NaN", "-20.0", "a NaN floor"),
        ("-60.0", "NaN", "a NaN ceiling"),
        ("-20.0", "-80.0", "an inverted pair"),
        ("-30.0", "-30.0", "a collapsed pair"),
        ("40.0", "60.0", "a pair right off the top of the scale"),
    ] {
        let mut state = fresh();
        state.picture.appearance.camera.yaw = 1.23;
        let saved = state.save_persist();
        let edited = saved
            .replace(
                &format!("floor_db:{:?},", state.picture.appearance.spectrum.floor_db),
                &format!("floor_db:{floor},"),
            )
            .replace(
                &format!("ceiling_db:{:?},", state.picture.appearance.spectrum.ceiling_db),
                &format!("ceiling_db:{ceiling},"),
            );
        assert_ne!(edited, saved, "{hint}: the level keys are not in the blob to edit");

        let mut restored = fresh();
        restored.load_persist(&edited);
        let cfg = restored.picture.appearance.spectrum;
        assert!(
            cfg.floor_db.is_finite() && cfg.ceiling_db.is_finite(),
            "{hint}: opened at {} .. {}",
            cfg.floor_db,
            cfg.ceiling_db,
        );
        assert!(
            cfg.ceiling_db - cfg.floor_db >= crate::LEVEL_RANGE_MIN_SPAN - 1e-3,
            "{hint}: opened on a {} dB window",
            cfg.ceiling_db - cfg.floor_db,
        );
        assert!(
            cfg.floor_db >= crate::LEVEL_MIN_DB && cfg.ceiling_db <= crate::LEVEL_MAX_DB,
            "{hint}: opened off the scale, at {} .. {}",
            cfg.floor_db,
            cfg.ceiling_db,
        );
        // And what reads out is what is drawn: the pane maps through this
        // same pair, so a repair that left the bar and the curve disagreeing
        // would be the silent break the loud one is preferred to.
        let level = crate::panes::spectral::axes::loudness_db(&cfg, cfg.ceiling_db, 0.0);
        assert!(level.is_finite(), "{hint}: the repaired pair still maps to {level}");
        assert_eq!(
            restored.picture.appearance.camera.yaw, 1.23,
            "{hint}: the rest of the blob still restores"
        );
    }
}

/// A double-click on a soft edge's bar puts it back to the FRESH pair, not to
/// the ends of its axis.
///
/// A range bar's own reset means "show the whole axis", which is the useful
/// thing to land on for a window onto something — the pitch range, the level.
/// It is the worst thing to land on here: the axis ends are the widest reach
/// there is at the softest it goes, so the gesture would replace a dialled
/// edge with the most extreme one instead of a neutral one. `edge_bar` takes
/// the fresh pair for exactly this.
///
/// It is also the gesture most likely to be aimed here by habit: every one of
/// these controls takes a six-digit number that gets captured out of a project
/// rather than dragged, and on a `ValueBar` a double-click opens the box to
/// type one.
///
/// Driven through the real widget rather than asserted on the mapping, since
/// what is being pinned is which of two resets the gesture reaches.
#[test]
fn a_double_click_on_a_soft_edge_restores_the_fresh_pair() {
    let ctx = super::probe::themed();
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 100.0));
    let fresh = harmonigraph_scene::ViewConfig::default();
    // Dialled somewhere else first, so landing on the fresh pair is a move.
    let (mut reach, mut fade) = (0.42f32, 0.1f32);
    let mut time = 0.0;
    let mut click = |reach: &mut f32, fade: &mut f32, events: Vec<egui::Event>| {
        time += 1.0 / 60.0;
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                crate::panes::edge_bar(
                    ui,
                    (reach, fade),
                    harmonigraph_scene::PLUS_SIZE_MAX,
                    "Arm length",
                    (fresh.plus_arm, fresh.plus_taper),
                    |v| format!("{v:.2}"),
                );
            },
        );
    };
    let at = egui::pos2(150.0, 10.0);
    let press = |pressed: bool| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    click(&mut reach, &mut fade, vec![]);
    for _ in 0..2 {
        let events = vec![egui::Event::PointerMoved(at), press(true), press(false)];
        click(&mut reach, &mut fade, events);
    }
    assert_eq!(
        (reach, fade),
        (fresh.plus_arm, fresh.plus_taper),
        "a double-click landed on ({reach}, {fade}) rather than the fresh pair",
    );
}

/// Every paired soft edge — the resting marker's arm and the taper on its
/// ends, and the lead a held note carries over the now-line —
/// opens on a pair its own bar can reach: a fade no wider than the reach it is
/// measured back from.
///
/// The picture does not care. Every one of the shaders caps the fade at the
/// reach it is taking out, so a fade dialled past that DRAWS as a fade over the
/// whole of it — which is exactly why the repair is safe to make, and why it
/// has to be made here rather than left to the draw path: each pair is one bar
/// reading out two points on one axis, and an unclamped fade puts its low end
/// off the bottom of that axis, where the bar would report a number the blob
/// does not hold.
#[test]
fn a_blob_naming_a_fade_wider_than_its_edge_opens_on_one_that_fits() {
    let mut state = fresh();
    state.picture.appearance.camera.yaw = 1.23;
    let saved = state.save_persist();
    let edited = saved
        .replace(
            &format!("roll_lead_fade:{:?},", state.picture.appearance.spectrum.roll_lead_fade),
            "roll_lead_fade:0.2,",
        )
        .replace(
            &format!("roll_lead:{:?},", state.picture.appearance.spectrum.roll_lead),
            "roll_lead:0.05,",
        )
        .replace(
            &format!("plus_taper:{:?},", state.picture.appearance.view.plus_taper),
            "plus_taper:0.5,",
        )
        .replace(
            &format!("plus_arm:{:?},", state.picture.appearance.view.plus_arm),
            "plus_arm:0.1,",
        );
    assert_ne!(edited, saved, "the edge keys are not in the blob to edit");

    let mut restored = fresh();
    restored.load_persist(&edited);
    assert_eq!(
        (
            restored.picture.appearance.spectrum.roll_lead,
            restored.picture.appearance.spectrum.roll_lead_fade
        ),
        (0.05, 0.05),
        "the lead's fade opened wider than the lead",
    );
    assert_eq!(
        (restored.picture.appearance.view.plus_arm, restored.picture.appearance.view.plus_taper),
        (0.1, 0.1),
        "a marker's taper opened longer than the arm it ends",
    );
    assert_eq!(restored.picture.appearance.camera.yaw, 1.23, "the rest of the blob still restores");
}

/// And a non-finite REACH opens on a drawable pair rather than taking the
/// editor down with it.
///
/// This is the one that has to be repaired rather than clamped, and the clamp
/// beside it is why: the reach is the fade's own upper bound, so a NaN reach
/// reaches `f32::clamp` as its `max`, and `clamp` asserts `min <= max` — which
/// NaN fails, panicking inside the host as the project opens. A NaN fade is
/// the milder half of the same input and survives its own clamp untouched,
/// leaving the bar to place a handle at a position that is not a number.
///
/// Both bars can produce neither shape, so a hand-edited or corrupted blob is
/// the only way in — the same standing this pair's neighbours have, and the
/// reason `a_blob_naming_an_undrawable_level_range_opens_on_a_drawable_one`
/// exists a few tests up.
///
/// The resting marker's arm is one such reach, and the order it is repaired in
/// is what keeps it safe: `sanitize` finishes the arm before it clamps the
/// taper to it, so `clamp` never sees a NaN as its `max`. A Shadow group's
/// width is a lone number rather than half of a pair and rides the same
/// repair — and it is edited through its own `width:…,spread:` anchor, which is
/// the shape only a `ShadowStyle` writes, so the edit cannot land on another
/// `…_width` key (`band_width`, say) the day the two hold the same number.
#[test]
fn a_blob_naming_a_nonsense_soft_edge_opens_on_a_drawable_one() {
    let cases: [(&str, &str, &str); 5] = [
        ("width", "NaN", "a NaN Shadow width"),
        ("roll_lead", "NaN", "a NaN lead"),
        ("roll_lead_fade", "inf", "an infinite lead fade"),
        // The marker's own pair.
        ("plus_arm", "NaN", "a NaN arm"),
        ("plus_taper", "inf", "an infinite taper"),
    ];
    for (key, value, hint) in cases {
        let mut state = fresh();
        state.picture.appearance.camera.yaw = 1.23;
        let saved = state.save_persist();
        let was = match key {
            "width" => state.picture.appearance.view.shadow.lattice_geometry.width,
            "roll_lead" => state.picture.appearance.spectrum.roll_lead,
            "plus_arm" => state.picture.appearance.view.plus_arm,
            "plus_taper" => state.picture.appearance.view.plus_taper,
            _ => state.picture.appearance.spectrum.roll_lead_fade,
        };
        // Anchored on what follows, for `width`: a `ShadowStyle` is the only
        // thing in the blob that writes a width with a spread after it, and
        // both of its groups are edited at once.
        let after = if key == "width" { "spread:" } else { "" };
        let edited =
            saved.replace(&format!("{key}:{was:?},{after}"), &format!("{key}:{value},{after}"));
        assert_ne!(edited, saved, "{hint}: `{key}` is not in the blob to edit");

        let mut restored = fresh();
        restored.load_persist(&edited);
        let view = &restored.picture.appearance.view;
        let cfg = &restored.picture.appearance.spectrum;
        for (name, v) in [
            ("lattice geometry width", view.shadow.lattice_geometry.width),
            ("lattice text width", view.shadow.lattice_text.width),
            ("spectral geometry width", view.shadow.spectral_geometry.width),
            ("spectral text width", view.shadow.spectral_text.width),
            ("roll_lead", cfg.roll_lead),
            ("roll_lead_fade", cfg.roll_lead_fade),
            ("plus_arm", view.plus_arm),
            ("plus_taper", view.plus_taper),
        ] {
            assert!(v.is_finite(), "{hint}: `{name}` opened at {v}");
        }
        assert!(
            cfg.roll_lead_fade <= cfg.roll_lead && view.plus_taper <= view.plus_arm,
            "{hint}: opened on a fade wider than its reach",
        );
        assert_eq!(
            restored.picture.appearance.camera.yaw, 1.23,
            "{hint}: the rest of the blob still restores"
        );
    }
}

/// The lead's RELEASE opens somewhere its own bar can reach, which for a
/// duration means finite and inside the bar's travel.
///
/// It is the odd one out among the lead's settings and gets its own blob for
/// that reason: the reach and the fade are two points on one axis and have each
/// other to be wrong about, where this is a lone number in seconds. What it can
/// still be is off the bar — a hand-edited blob asking for a lead that outlives
/// its note by a minute — or not a number at all, where the draw path refuses
/// it silently (`panes::spectral::roll::lead_alpha` compares against a NaN and
/// simply draws no lead) and the pane would then be at a value the file does not
/// hold, indefinitely, since nothing rewrites the key until someone drags that
/// bar.
#[test]
fn a_blob_naming_a_lead_release_off_its_own_bar_opens_on_one_that_fits() {
    for (value, want, hint) in [
        ("99.0", crate::ROLL_LEAD_RELEASE_MAX, "a release of a minute and a half"),
        ("-1.0", 0.0, "a negative release"),
        ("NaN", SpectrumConfig::default().roll_lead_release, "a release that is not a number"),
    ] {
        let mut state = fresh();
        state.picture.appearance.camera.yaw = 1.23;
        let saved = state.save_persist();
        let was = state.picture.appearance.spectrum.roll_lead_release;
        let edited = saved.replace(
            &format!("roll_lead_release:{was:?},"),
            &format!("roll_lead_release:{value},"),
        );
        assert_ne!(edited, saved, "{hint}: `roll_lead_release` is not in the blob to edit");

        let mut restored = fresh();
        restored.load_persist(&edited);
        assert_eq!(
            restored.picture.appearance.spectrum.roll_lead_release, want,
            "{hint}: opened at {}",
            restored.picture.appearance.spectrum.roll_lead_release,
        );
        assert_eq!(
            restored.picture.appearance.camera.yaw, 1.23,
            "{hint}: the rest of the blob still restores"
        );
    }
}

/// The curve's Attack and Release open somewhere their own bars can reach.
///
/// The case a finiteness check alone lets through is the one worth pinning: a
/// huge time is a perfectly good `f32`, and the coefficient it asks for
/// (`hop_alpha`) rounds to 0, so the curve holds whatever it last had for as
/// long as the plugin is open — while the spectrogram beside it, which reads
/// the raw columns rather than the smoothed ones, carries on drawing. The pane
/// would then be at a value the file does not hold until someone drags that
/// bar.
#[test]
fn a_blob_naming_a_curve_time_off_its_own_bar_opens_on_one_that_fits() {
    for (key, fresh_value) in [
        ("attack", SpectrumConfig::default().attack),
        ("release", SpectrumConfig::default().release),
    ] {
        for (value, want, hint) in [
            ("1e10", crate::config::BALLISTICS_MAX, "a time that freezes the curve"),
            ("-1.0", 0.0, "a negative time"),
            ("NaN", fresh_value, "a time that is not a number"),
        ] {
            let mut state = fresh();
            state.picture.appearance.camera.yaw = 1.23;
            let saved = state.save_persist();
            let edited =
                saved.replace(&format!("{key}:{fresh_value:?},"), &format!("{key}:{value},"));
            assert_ne!(edited, saved, "{hint}: `{key}` is not in the blob to edit");

            let mut restored = fresh();
            restored.load_persist(&edited);
            let got = if key == "attack" {
                restored.picture.appearance.spectrum.attack
            } else {
                restored.picture.appearance.spectrum.release
            };
            assert_eq!(got, want, "{hint}: `{key}` opened at {got}");
            assert_eq!(
                restored.picture.appearance.camera.yaw, 1.23,
                "{hint}: the rest of the blob still restores"
            );
        }
    }
}

#[test]
fn analyzer_scalars_are_normalized_before_any_settings_are_drawn() {
    let previous: SpectrumConfig =
        ron::from_str("(analyzer_min_brightness:0.9, floor_db:-90.0)").unwrap();
    assert_eq!(previous.keyline_lift, SpectrumConfig::default().keyline_lift);
    assert_eq!(previous.floor_db, -90.0);
    let outlined: SpectrumConfig = ron::from_str("(keyline_lift:0.9)").unwrap();
    assert_eq!(outlined.keyline_lift, 0.9);
    type Field = fn(&mut SpectrumConfig) -> &mut f32;
    let fields: [(Field, f32, f32); 8] = [
        (|cfg| &mut cfg.tilt, -6.0, 0.0),
        (|cfg| &mut cfg.keyline_lift, 0.0, 1.0),
        (|cfg| &mut cfg.backdrop_strength, 0.0, 1.0),
        (|cfg| &mut cfg.backdrop_height, 0.05, 1.0),
        (|cfg| &mut cfg.backdrop_gap, 0.0, 7.0),
        (|cfg| &mut cfg.roll_fraction, 0.0, 1.0),
        (|cfg| &mut cfg.roll_seconds, ROLL_SECONDS_MIN, ROLL_SECONDS_MAX),
        (|cfg| &mut cfg.roll_thickness, 0.2, 2.0),
    ];
    for (field, min, max) in fields {
        let default = *field(&mut SpectrumConfig::default());
        for (value, expected) in [
            (min - 1.0, min),
            (max + 1.0, max),
            (f32::NAN, default),
            (f32::INFINITY, default),
            (f32::NEG_INFINITY, default),
        ] {
            let mut state = fresh();
            *field(&mut state.picture.appearance.spectrum) = value;
            let mut restored = fresh();
            assert!(restored.load_persist(&state.save_persist()));
            assert_eq!(*field(&mut restored.picture.appearance.spectrum), expected);
            let normalized = restored.save_persist();
            assert!(restored.load_persist(&normalized));
            assert_eq!(restored.save_persist(), normalized, "normalization is idempotent");
        }
    }
    // The stripe spacing is whole pixels, so a fraction lands on the nearest.
    let mut state = fresh();
    state.picture.appearance.spectrum.backdrop_gap = 3.4;
    let mut restored = fresh();
    assert!(restored.load_persist(&state.save_persist()));
    assert_eq!(restored.picture.appearance.spectrum.backdrop_gap, 3.0);
}

/// Retired outer-octave settings do not prevent restoring the appearance.
#[test]
fn a_blob_with_outer_octave_settings_keeps_its_count_and_camera() {
    let mut state = fresh();
    state.picture.appearance.camera.yaw = 1.23;
    state.picture.appearance.view.octave_count = 7;
    let saved = state.save_persist();
    let old = saved.replace(
        "octave_count:7,",
        "octave_count:7,octave_extras:2,octave_extra_size:0.4,octave_extra_blend:0.5,",
    );
    assert_ne!(old, saved);
    let mut restored = fresh();
    assert!(restored.load_persist(&old));
    assert_eq!(restored.picture.appearance.view.octave_count, 7);
    assert_eq!(restored.picture.appearance.camera.yaw, 1.23);
}

/// The render frame round-trips its side and the split beside it, through
/// editor persistence.
#[test]
fn a_render_frame_round_trips() {
    let mut state = fresh();
    state.picture.appearance.camera.yaw = 1.23;
    state.picture.appearance.render.frame.lattice = LatticeSide::Bottom;
    state.picture.appearance.render.frame.split = 0.42;
    let saved = state.save_persist();
    assert!(saved.contains("lattice:Bottom"), "the side is what gets written");

    let mut restored = fresh();
    restored.load_persist(&saved);
    assert_eq!(restored.picture.appearance.render.frame.lattice, LatticeSide::Bottom);
    assert_eq!(restored.picture.appearance.render.frame.split, 0.42);
    assert_eq!(restored.picture.appearance.camera.yaw, 1.23, "rest of the blob still restores");
}

#[test]
fn corrupt_persist_is_ignored() {
    let mut state = fresh();
    let default_distance = state.picture.appearance.camera.distance;
    assert!(!state.load_persist("not json at all"), "a corrupt blob is not applied");
    assert_eq!(state.picture.appearance.camera.distance, default_distance);
    assert!(!collapsed(&state, panes::Tab::Console), "the explanation is visible");
}

/// A refused blob SAYS SO. A refusal costs the whole document — dock,
/// camera, view, spectrum and render at once — and a project reopening at
/// defaults with nothing written anywhere reads as data loss rather than as a
/// break someone chose. The dropped-variant refusals are
/// [`a_dropped_variant_refuses_the_whole_document_and_says_so`]; these are the
/// version floor's and the appearance version's.
#[test]
fn a_refused_blob_says_why() {
    let saved = fresh().save_persist();
    let stale = saved.replacen(
        &format!("version:{UI_PERSIST_VERSION}"),
        &format!("version:{}", UI_PERSIST_VERSION - 1),
        1,
    );
    let mut older = fresh();
    assert!(!older.load_persist(&stale), "a blob below the floor is not applied");
    assert!(
        older.picture.runtime.console.lines().any(|line| line.contains("below the floor")),
        "the floor's refusal was silent",
    );
    assert!(!collapsed(&older, panes::Tab::Console), "the version refusal opens its report");
    let unsupported = saved.replace("appearance:(version:1", "appearance:(version:0");
    assert_ne!(unsupported, saved);
    let mut restored = fresh();
    set_console_collapsed(&mut restored, true);
    let before = restored.save_persist();
    assert!(!restored.load_persist(&unsupported));
    assert!(!collapsed(&restored, panes::Tab::Console), "appearance refusal opens its report");
    set_console_collapsed(&mut restored, true);
    assert_eq!(
        restored.save_persist(),
        before,
        "appearance refusal must apply no workspace state beyond revealing Console",
    );
    assert!(restored
        .picture
        .runtime
        .console
        .lines()
        .any(|line| line.contains("appearance version 0 is unsupported")));
}

#[test]
fn spectrum_config_round_trips_through_persist() {
    let mut state = fresh();
    state.picture.appearance.spectrum.floor_db = -48.0;
    state.picture.appearance.spectrum.ceiling_db = -12.0;
    state.picture.appearance.spectrum.volume_floor_db = -72.0;
    state.picture.appearance.spectrum.volume_ceiling_db = -18.0;
    state.picture.appearance.spectrum.window = SpectrumWindow::Precise;
    state.picture.appearance.spectrum.low_midi = 40.5;
    state.picture.appearance.spectrum.show_spectrogram = true;
    // A gradient no preset writes, so what round-trips is the numbers and not a
    // name that happens to rebuild them — a bend included, which no preset has.
    state.picture.appearance.spectrum.spectrogram_gradient = harmonigraph_scene::Gradient {
        hue_start: 137.5,
        hue_span: -85.25,
        lightness: 44.0,
        lightness_ramp: 71.5,
        chroma: 0.375,
        chroma_ramp: -0.25,
        bend: harmonigraph_scene::Bend {
            at: 0.9,
            share: 0.15,
            lightness: false,
            ..Default::default()
        },
    };
    let saved = state.save_persist();

    let mut restored = fresh();
    restored.load_persist(&saved);
    assert_eq!(restored.picture.appearance.spectrum.floor_db, -48.0);
    assert_eq!(restored.picture.appearance.spectrum.ceiling_db, -12.0);
    assert_eq!(restored.picture.appearance.spectrum.volume_floor_db, -72.0);
    assert_eq!(restored.picture.appearance.spectrum.volume_ceiling_db, -18.0);
    assert_eq!(restored.picture.appearance.spectrum.window, SpectrumWindow::Precise);
    // A range off the C boundaries survives, which the octave pair could not
    // have expressed at all.
    assert_eq!(restored.picture.appearance.spectrum.low_midi, 40.5);
    assert!(restored.picture.appearance.spectrum.show_spectrogram);
    assert_eq!(
        restored.picture.appearance.spectrum.spectrogram_gradient,
        state.picture.appearance.spectrum.spectrogram_gradient,
        "every knob of the heatmap's gradient, not just the ones a preset moves",
    );
}

/// A range saved while the axis ran MIDI 12..132 (16 Hz to 16.7 kHz) starts
/// below the 20 Hz floor the axis has now. Drawing it would leave a band with
/// no buckets behind it, so loading fits the range to the axis that exists —
/// and only where it has to: 132 is still a pitch this axis covers.
#[test]
fn a_pitch_range_off_the_current_axis_is_pulled_back_onto_it() {
    use harmonigraph_core::spectrum::{SPECTRUM_MAX_MIDI, SPECTRUM_MIN_MIDI};
    let restore = |low: &str, high: &str| {
        let mut state = fresh();
        state.picture.appearance.spectrum.low_midi = 60.0;
        state.picture.appearance.spectrum.high_midi = 72.0;
        let saved = state
            .save_persist()
            .replace("low_midi:60.0", &format!("low_midi:{low}"))
            .replace("high_midi:72.0", &format!("high_midi:{high}"));
        let mut restored = fresh();
        restored.load_persist(&saved);
        (
            restored.picture.appearance.spectrum.low_midi,
            restored.picture.appearance.spectrum.high_midi,
        )
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
    let mut state = fresh();
    state.picture.appearance.spectrum.show_spectrogram = false;
    let saved = state.save_persist();
    let old = saved.replace("show_spectrogram:false,", "");
    assert_ne!(old, saved, "the field must have been there to strip");

    let mut restored = fresh();
    restored.load_persist(&old);
    assert!(
        restored.picture.appearance.spectrum.show_spectrogram,
        "a missing field must fall back to the struct's own default, not bool::default()"
    );
    // An explicit `false` is a choice, not an absence, and still round-trips.
    let mut restored = fresh();
    restored.load_persist(&saved);
    assert!(!restored.picture.appearance.spectrum.show_spectrogram);
}

/// The mirror of the case above: a field the blob is MISSING must not sink it
/// either, and must come back as what a fresh install has rather than as a bare
/// `0`/`false`.
///
/// What makes it worth pinning is what the alternative COSTS. No field of
/// [`SpectrumConfig`](crate::SpectrumConfig) carries a fallback of its own, so
/// without the struct's container-level `#[serde(default)]` a blob missing any
/// one key fails to parse — and a parse that fails loses the WHOLE UI state
/// rather than that key: the camera, the dock and the view go with it. The
/// container-level attribute is what closes that, every field falling back to
/// `impl Default`'s value, so a missing key costs only itself.
///
/// Pinned per field rather than once, because the hazard is per field: nothing
/// at a declaration says whether it has a fallback, so the next field added is
/// covered silently and the next one REMOVED is the one that would sink a saved
/// project.
#[test]
fn a_persist_blob_missing_a_spectrum_field_keeps_the_rest_of_the_blob() {
    let defaults = crate::SpectrumConfig::default();
    for key in [
        format!("release:{:?},", defaults.release),
        format!("floor_db:{:?},", defaults.floor_db),
        format!("roll_thickness:{:?},", defaults.roll_thickness),
        format!("volume_floor_db:{:?},", defaults.volume_floor_db),
        format!("window:{:?},", defaults.window),
    ] {
        let mut state = fresh();
        // A non-default elsewhere in the blob, so "the blob survived" is
        // distinguishable from "it sank and everything reverted".
        state.picture.appearance.view.max_sevens = 3;
        let saved = state.save_persist();
        let without = saved.replacen(key.as_str(), "", 1);
        assert_ne!(without, saved, "{key:?} must be in the blob to drop");

        let mut restored = fresh();
        restored.load_persist(&without);
        assert_eq!(
            restored.picture.appearance.view.max_sevens, 3,
            "dropping {key:?} must cost that key alone, not the whole blob",
        );
        assert_eq!(
            restored.picture.appearance.spectrum, defaults,
            "and the config it belongs to must load at the fresh-install values",
        );
    }
}

/// Workspace settings stay alongside one nested appearance document.
#[test]
fn the_persist_blob_carries_exactly_these_top_level_keys() {
    // UiPersist's fields, in declaration order.
    const KEYS: &[&str] = &[
        "version",
        "layout",
        "analyzer_regions",
        "spiral",
        "folded_sections",
        "appearance",
        "camera_presets",
        "fps_cap",
        "ui_scale",
        "skin_dials",
        "perf_pos",
        "show_perf",
        "show_perf_detail",
    ];

    let saved = fresh().save_persist();
    let keys: Vec<String> = top_level_pairs(&saved).into_iter().map(|(key, _)| key).collect();
    assert_eq!(keys, KEYS, "the persist blob's top-level keys have moved");
}
/// The same property for EVERY section that carries the attribute, not just
/// `render`: dropping one costs that section alone.
///
/// Swept rather than pinned one at a time, because nothing at a declaration
/// says whether `#[serde(default)]` is there — `camera` and `view` went
/// without it for a while precisely because the omission is invisible, and a
/// hand-authored `--appearance` file setting one thing omits most of the rest.
/// A section added without the attribute fails here rather than the day a
/// blob is short of it.
///
/// `dock` is the exception and is not swept: it has no `impl Default`, so a
/// blob with no dock has no layout to restore and refusing the document is
/// the honest answer.
#[test]
fn a_persist_blob_missing_any_one_section_keeps_the_rest() {
    let mut state = fresh();
    // A witness in a section that is never the one dropped below, so "the
    // blob survived" is distinguishable from "it sank and everything
    // reverted". The camera is not swept for that reason.
    state.picture.appearance.camera.yaw = 1.23;
    state.picture.appearance.view.max_sevens = 3;
    let saved = state.save_persist();

    for (key, _) in top_level_pairs(&saved) {
        if key == "version" || key == "appearance" {
            continue;
        }
        let kept: Vec<String> = top_level_pairs(&saved)
            .into_iter()
            .filter(|(k, _)| *k != key)
            .map(|(_, text)| text)
            .collect();
        let without = format!("({})", kept.join(","));
        assert_ne!(without, saved, "{key:?} must be in the blob to drop");

        let mut restored = fresh();
        assert!(
            restored.load_persist(&without),
            "dropping {key:?} sank the whole document instead of costing itself",
        );
        assert_eq!(
            restored.picture.appearance.camera.yaw, 1.23,
            "dropping {key:?} cost the camera too"
        );
        // Every top-level value in `saved` is the fresh one, so a dropped key
        // must come back exactly as it was.
        assert_eq!(restored.save_persist(), saved, "{key:?} came back at a non-fresh value");
    }
}

#[test]
fn workspace_edits_do_not_change_recorded_appearance() {
    let mut state = fresh();
    state.picture.appearance.camera.yaw = 1.23;
    state.picture.appearance.view.max_sevens = 3;
    state.picture.appearance.spectrum.low_midi = 40.5;
    state.picture.appearance.render.short_edge = 2160;
    let appearance = state.picture.appearance.serialize();
    let editor = state.save_persist();
    state.workspace.layout = workspace::Layout::solo(crate::panes::Tab::Console);
    state.workspace.interaction.spiral.zoom = 2.75;
    state.workspace.layout.right.lattice = 320.0;
    state.workspace.interaction.folded_sections.insert("System/Performance".into());
    state.workspace.interaction.ui_scale = 1.25;
    state.workspace.interaction.fps_cap = Some(30.0);
    assert_ne!(state.save_persist(), editor);
    assert_eq!(state.picture.appearance.serialize(), appearance);
    let mut restored = fresh();
    assert!(restored.load_persist(&state.save_persist()));
    assert_eq!(restored.picture.appearance.serialize(), appearance);
    // An editor-only enum can make the enclosing save unreadable without
    // affecting the separately captured appearance at all.
    let broken_workspace = state.save_persist().replace("Console", "RetiredConsole");
    assert!(!restored.load_persist(&broken_workspace));
    assert_eq!(AppearanceDocument::parse(&appearance).unwrap().serialize(), appearance);
}

/// Missing groups use their own defaults without costing the other groups.
#[test]
fn an_appearance_missing_any_one_group_keeps_the_rest() {
    let mut state = fresh();
    state.picture.appearance.camera.yaw = 1.23;
    state.picture.appearance.view.max_sevens = 3;
    state.picture.appearance.spectrum.low_midi = 40.5;
    state.picture.appearance.render.short_edge = 2160;
    let saved = state.picture.appearance.serialize();
    let defaults = AppearanceDocument::default().serialize();
    let pairs = top_level_pairs(&saved);
    for (missing, _) in &pairs {
        let without = format!(
            "({})",
            pairs
                .iter()
                .filter(|(key, _)| key != missing)
                .map(|(_, pair)| pair.as_str())
                .collect::<Vec<_>>()
                .join(",")
        );
        let restored = AppearanceDocument::parse(&without).expect("missing group defaults");
        for (key, pair) in top_level_pairs(&restored.serialize()) {
            let source = if &key == missing { &defaults } else { &saved };
            let expected = top_level_pairs(source).into_iter().find(|(k, _)| k == &key).unwrap().1;
            assert_eq!(pair, expected, "dropping {missing} changed {key}");
        }
    }
}

/// The Spiral pane's framing round-trips, and a hand-edited one comes back
/// drawable.
///
/// Persisted for the reason the lattice's camera is — a framing is dialled in by
/// hand and comes back when the editor reopens — so what has to hold is that the pair
/// SURVIVES rather than being re-derived from the fit, and that a nonsense one
/// cannot reach the pane: both fields multiply the geometry it paints, and NaN
/// geometry is a panic inside egui's tessellator.
#[test]
fn persist_round_trips_the_spiral_framing() {
    let mut state = fresh();
    // A zoom off both ends of the range and a look off both axes, so a field
    // dropped or transposed on the way through shows up.
    state.workspace.interaction.spiral =
        crate::panes::spiral::SpiralView { zoom: 2.75, look: glam::vec2(0.4, -0.6) };

    let mut restored = fresh();
    assert!(restored.load_persist(&state.save_persist()));
    assert_eq!(restored.workspace.interaction.spiral.zoom, 2.75);
    assert_eq!(restored.workspace.interaction.spiral.look, glam::vec2(0.4, -0.6));

    // And the repair on the way in, which is `load_persist`'s call rather than
    // the pane's: a blob nothing but a text editor could have written.
    let saved = state.save_persist();
    let edited = replace_pair(&saved, "zoom", "2.75", "NaN");
    assert_ne!(edited, saved, "`zoom` is not in the blob to edit");
    let mut restored = fresh();
    assert!(restored.load_persist(&edited));
    assert!(
        restored.workspace.interaction.spiral.zoom.is_finite(),
        "a NaN zoom opened at {}",
        restored.workspace.interaction.spiral.zoom,
    );
}

/// Dropping any one key from INSIDE the `spiral` section costs that key alone:
/// the rest of the blob loads, the section's other field keeps the value the blob
/// names, and the dropped one comes back at the fresh-install value.
///
/// One layer in from [`a_persist_blob_missing_any_one_section_keeps_the_rest`],
/// and that is the whole reason it exists: sweeping whole sections exercises
/// `UiPersist`'s container-level attribute, where this is the only thing that
/// asks after `SpiralView`'s container-level one — the attribute nothing at a
/// declaration says is there.
///
/// The input is `spiral: (zoom: 3.0)` and no `look`, which is what a person
/// writing a framing into a saved blob by hand writes: the field they came to
/// change. Without the attribute that blob does not cost `look` — it sinks the
/// whole document, dock and camera with it, and nothing but this would fail.
#[test]
fn a_persist_blob_missing_any_one_spiral_key_keeps_the_rest() {
    let mut state = fresh();
    // A witness outside the section, so "the blob survived" is distinguishable
    // from "it sank and every section reverted together".
    state.picture.appearance.camera.yaw = 1.23;
    // Both fields off their fresh values, and the look off both axes: a field
    // that came back from the wrong place is only visible against a value the
    // default is not.
    state.workspace.interaction.spiral =
        crate::panes::spiral::SpiralView { zoom: 2.75, look: glam::vec2(0.4, -0.6) };
    let saved = state.save_persist();

    let whole = top_level_pairs(&saved)
        .into_iter()
        .find_map(|(key, text)| (key == "spiral").then_some(text))
        .expect("the blob carries a spiral section");
    // The section's own pairs, through the same depth-aware split: `look` is a
    // nested struct, so a comma split would cut it in half.
    let inner = whole.trim_start_matches("spiral:");
    let pairs = top_level_pairs(inner);
    assert_eq!(pairs.len(), 2, "the probe must see the whole framing, got {pairs:?}");

    let opened = crate::panes::spiral::SpiralView::default();
    for (key, _) in &pairs {
        let kept: Vec<&str> =
            pairs.iter().filter(|(k, _)| k != key).map(|(_, text)| text.as_str()).collect();
        let without = replace_pair(&saved, "spiral", inner, &format!("({})", kept.join(",")));
        assert_ne!(without, saved, "the spiral's {key:?} must be in the blob to drop");

        let mut restored = fresh();
        assert!(
            restored.load_persist(&without),
            "dropping the spiral's {key:?} sank the whole document instead of costing itself",
        );
        assert_eq!(
            restored.picture.appearance.camera.yaw, 1.23,
            "dropping the spiral's {key:?} cost the camera too"
        );
        let framing = state.workspace.interaction.spiral;
        let want = match key.as_str() {
            "zoom" => (opened.zoom, framing.look),
            "look" => (framing.zoom, opened.look),
            other => panic!("the framing grew a {other:?} field this sweep does not name"),
        };
        let restored = restored.workspace.interaction.spiral;
        assert_eq!(
            (restored.zoom, restored.look),
            want,
            "dropping the spiral's {key:?} did not cost that key alone",
        );
    }
}

#[test]
fn persist_round_trips_the_frame_rate_cap() {
    let mut state = fresh();
    // Not one of the button values, so it proves the number round-trips
    // rather than being re-derived from a default.
    state.workspace.interaction.fps_cap = Some(45.0);

    let mut restored = fresh();
    restored.load_persist(&state.save_persist());
    assert_eq!(restored.workspace.interaction.fps_cap, Some(45.0));
}

/// A blob saved before the control existed loads at the design size. `f32`'s
/// own serde default is 0.0 — a scale of nothing, and every one of those
/// projects.
#[test]
fn a_blob_without_a_ui_scale_loads_at_the_design_size() {
    let mut state = fresh();
    state.workspace.interaction.ui_scale = 0.75;
    let saved = state.save_persist();
    let mut back = fresh();
    back.load_persist(&saved);
    assert_eq!(back.workspace.interaction.ui_scale, 0.75, "the scale did not round-trip");

    // The same blob with the field taken back out, which is what every project
    // saved before this looks like.
    let stripped =
        saved.split(",ui_scale:").next().expect("the blob names the field").to_owned() + ")";
    let mut older = fresh();
    older.load_persist(&stripped);
    assert_eq!(
        older.workspace.interaction.ui_scale, 1.0,
        "a blob with no scale did not load at the design size"
    );
}

/// An out-of-range scale — only a hand-edited blob can produce one — is
/// clamped rather than drawn at.
#[test]
fn an_impossible_ui_scale_is_clamped() {
    let ctx = super::probe::themed();
    // A nonsense value falls back to the design size rather than to the end of
    // the range it points at: an infinity is not a request for the largest
    // chrome available, it is a blob that has lost the number.
    for (asked, expected) in [(0.01f32, 0.7f32), (99.0, 1.5), (f32::NAN, 1.0), (f32::INFINITY, 1.0)]
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
        let mut state = fresh();
        // Off the defaults, so the splice has something to name and a range
        // that survives proves it survived rather than matching by luck.
        state.picture.appearance.spectrum.low_midi = 40.5;
        state.picture.appearance.spectrum.high_midi = 90.25;
        let saved = state.save_persist();
        let value = if end == "low_midi" { 40.5f32 } else { 90.25f32 };
        let broken = saved.replacen(&format!("{end}:{value:?}"), &format!("{end}:NaN"), 1);
        assert_ne!(broken, saved, "the {end} splice must land for this to test anything");

        let mut restored = fresh();
        restored.load_persist(&broken);
        let (low, high) = (
            restored.picture.appearance.spectrum.low_midi,
            restored.picture.appearance.spectrum.high_midi,
        );
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

/// The heatmap's gradient comes back drawable, the third field on this door
/// after the pitch range and the UI scale — and the one whose repair is easiest
/// to leave out, `Gradient` having a `sanitized` of its own that the DRAW path
/// calls anyway.
///
/// That is exactly why it needs a test rather than an argument. The picture is
/// right either way, because `with_lut` sanitizes at the table and the bars
/// sanitize before they paint; what is wrong without the repair is that the
/// FILE keeps a pair the picture is not at, indefinitely, and nothing rewrites
/// it until someone drags that bar. CLAUDE.md is the line this is held to: "The
/// value on screen must still be the value the file holds."
///
/// Two shapes, because they fail differently. A ramp wider than its middle
/// leaves is a legal pair of floats that names an illegal picture — sanitize
/// pulls it in. A NaN is not a number at all, and it is the one that would ride
/// into a color conversion and out into the instance buffer unannounced.
#[test]
fn a_blob_with_a_nonsense_heatmap_gradient_loads_at_a_drawable_one() {
    for (key, broken) in [("lightness", "5.0"), ("chroma", "NaN")] {
        let mut state = fresh();
        // A gradient off the defaults, and one sanitize leaves alone, so the
        // splice has something to name and the untouched knobs prove they
        // survived rather than matching a fresh install by luck.
        state.picture.appearance.spectrum.spectrogram_gradient = harmonigraph_scene::Gradient {
            hue_start: 137.5,
            hue_span: -85.25,
            lightness: 44.0,
            lightness_ramp: 71.5,
            chroma: 0.375,
            chroma_ramp: -0.25,
            ..Default::default()
        };
        let saved = state.save_persist();
        let was = if key == "lightness" { "44.0" } else { "0.375" };
        let spliced = saved.replacen(&format!("{key}:{was}"), &format!("{key}:{broken}"), 1);
        assert_ne!(spliced, saved, "the {key} splice must land for this to test anything");

        let mut restored = fresh();
        restored.load_persist(&spliced);
        let g = restored.picture.appearance.spectrum.spectrogram_gradient;
        assert_eq!(g.sanitized(), g, "{key}:{broken} left the file holding {g:?}");
        // And the knobs the splice did not touch keep what the blob said, so
        // the repair cannot pass by resetting the whole gradient.
        assert_eq!(g.hue_start, 137.5, "{key}:{broken}: an untouched knob was reset");
        assert_eq!(g.hue_span, -85.25, "{key}:{broken}: an untouched knob was reset");
    }
}

#[test]
fn a_blob_older_than_the_version_floor_is_refused_whole() {
    // The refusal is what lets the migrations for older formats be deleted
    // rather than carried forever, and it costs a real project its settings:
    // the plugin's CLAP id gates everything below version 2, but nothing
    // gates a blob one bump behind, which a project saved by the previous build
    // is. See `load_persist` for why that price is paid rather than shimmed.
    let mut state = fresh();
    state.picture.appearance.camera.yaw = 1.23;
    state.picture.appearance.view.max_sevens = 3;
    let saved = state.save_persist();
    assert!(saved.contains(&format!("version:{UI_PERSIST_VERSION}")), "saves at the floor");

    let stale = saved.replacen(
        &format!("version:{UI_PERSIST_VERSION}"),
        &format!("version:{}", UI_PERSIST_VERSION - 1),
        1,
    );
    let mut restored = fresh();
    restored.load_persist(&stale);
    // Untouched, not partially applied: a refused blob must not leave the
    // camera from one era beside a dock from another.
    let defaults = fresh();
    assert_eq!(restored.picture.appearance.camera.yaw, defaults.picture.appearance.camera.yaw);
    assert_eq!(
        restored.picture.appearance.view.max_sevens,
        defaults.picture.appearance.view.max_sevens
    );

    // And the same blob at the floor still loads, so the test above is
    // measuring the version rather than a blob that was broken anyway.
    let mut current = fresh();
    current.load_persist(&saved);
    assert_eq!(current.picture.appearance.camera.yaw, 1.23);
    assert_eq!(current.picture.appearance.view.max_sevens, 3);
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
    let mut state = fresh();
    // A blob from before the septimal comma existed: its keys are stripped,
    // so both `marvel` and `marvel_auto` take their engaged fresh defaults.
    let full = state.save_persist();
    let without_mode = full.replace("marvel:true,", "");
    assert_ne!(without_mode, full, "mode removal must have hit");
    let saved = without_mode.replace("marvel_auto:true,", "");
    assert_ne!(saved, without_mode, "detect removal must have hit");

    // This session has already judged the tuning it is sitting at.
    state.picture.runtime.config_reducer.sync_display(
        harmonigraph_core::Tuning::default(),
        Default::default(),
        Default::default(),
    );
    assert!(state.picture.runtime.config_reducer.judged().iter().all(Option::is_some));
    state.load_persist(&saved);
    assert_eq!(
        state.picture.runtime.config_reducer.judged(),
        [None; Comma::COUNT],
        "a loaded project must be judged on its own terms",
    );
    assert!(state.picture.appearance.view.marvel_auto, "and the missing detect key still opts in");
}

/// Dropping any one key from a serialized view costs THAT KEY alone, and the
/// value it comes back with is the fresh-install one.
///
/// [`ViewConfig`](harmonigraph_scene::ViewConfig) carries a container-level
/// `#[serde(default)]`, so `impl Default` is the single source of every
/// field's fallback. That is the whole arrangement, and this is what holds it:
/// nothing at a declaration says whether a field can survive being absent, so
/// the property is probed from the outside instead — rebuild the blob without
/// one key at a time, which is exactly the shape a hand-edited RON or a
/// `--appearance` file can arrive in, and reload.
///
/// A field that fails here either sank the whole view (no fallback) or came
/// back as something other than the fresh value (a second default hiding
/// behind a field-level attribute). Both are the same bug one layer up, where
/// `a_persist_blob_missing_a_spectrum_field_keeps_the_rest_of_the_blob` pins
/// it for `SpectrumConfig`: a missing key must cost only itself.
#[test]
fn a_view_missing_any_one_key_reloads_at_the_fresh_value() {
    let fresh = harmonigraph_scene::ViewConfig::default();
    let full = ron::to_string(&fresh).expect("a view serializes");
    let pairs = top_level_pairs(&full);
    assert!(pairs.len() > 40, "the probe must see the whole struct, got {}", pairs.len());

    for (key, _) in &pairs {
        let kept: Vec<&str> =
            pairs.iter().filter(|(k, _)| k != key).map(|(_, text)| text.as_str()).collect();
        let without = format!("({})", kept.join(","));
        let loaded = ron::from_str::<harmonigraph_scene::ViewConfig>(&without)
            .unwrap_or_else(|e| panic!("dropping {key:?} sank the whole view: {e}"));
        assert_eq!(
            ron::to_string(&loaded).expect("a view serializes"),
            full,
            "dropping {key:?} did not come back at the fresh-install value",
        );
    }
}

#[test]
fn lattice_map_indicator_visibility_round_trips_and_defaults_on() {
    let mut state = fresh();
    state.picture.appearance.view.show_map_indicators = false;
    let saved = state.save_persist();

    let mut restored = fresh();
    restored.load_persist(&saved);
    assert!(
        !restored.picture.appearance.view.show_map_indicators,
        "a project that hid map indicators must keep them hidden"
    );

    let old = saved.replacen("show_map_indicators:false,", "", 1);
    assert_ne!(old, saved, "the visibility-key cut must land");
    restored.load_persist(&old);
    assert!(
        restored.picture.appearance.view.show_map_indicators,
        "a project from before the visibility setting existed must keep showing map indicators"
    );
}

#[test]
fn a_shadow_endpoint_missing_any_one_group_keeps_the_other_three() {
    let mut endpoint = harmonigraph_scene::ShadowSettings::default();
    for (index, group) in endpoint.groups_mut().into_iter().enumerate() {
        group.width = 0.1 + index as f32 * 0.1;
        group.depth = 0.2 + index as f32 * 0.1;
    }
    let full = ron::to_string(&endpoint).expect("Shadow settings serialize");
    let pairs = top_level_pairs(&full);
    assert_eq!(pairs.len(), 4, "the persisted endpoint is not exactly four groups: {full}");
    assert!(!full.contains("spiral"), "the spiral grew a distinct persisted group: {full}");

    for (missing, _) in &pairs {
        let kept: Vec<&str> = pairs
            .iter()
            .filter(|(name, _)| name != missing)
            .map(|(_, text)| text.as_str())
            .collect();
        let loaded: harmonigraph_scene::ShadowSettings =
            ron::from_str(&format!("({})", kept.join(",")))
                .unwrap_or_else(|e| panic!("dropping {missing} sank the endpoint: {e}"));
        // A missing group comes back at the FRESH endpoint's value for that
        // group, not at a bare `ShadowStyle::default()`: the container-level
        // `serde(default)` fills from `ShadowSettings::default()`, whose four
        // groups differ.
        let fresh = harmonigraph_scene::ShadowSettings::default();
        for (name, group) in [
            ("lattice_geometry", loaded.lattice_geometry),
            ("lattice_text", loaded.lattice_text),
            ("spectral_geometry", loaded.spectral_geometry),
            ("spectral_text", loaded.spectral_text),
        ] {
            let source = if name == missing { &fresh } else { &endpoint };
            let want = match name {
                "lattice_geometry" => source.lattice_geometry,
                "lattice_text" => source.lattice_text,
                "spectral_geometry" => source.spectral_geometry,
                _ => source.spectral_text,
            };
            assert_eq!(group, want, "dropping {missing} changed {name}");
        }
    }
}

/// The other half of that pair, one layer in: a group the blob CARRIES with one
/// of its FIELDS missing fills that field from the bare
/// [`ShadowStyle::default`](harmonigraph_scene::ShadowStyle::default), not from
/// that group's entry in `ShadowSettings::default()` — the answer the sweep
/// above pins for a whole group gone. The two are entitled to differ (see the
/// rationale on `impl Default for ShadowStyle`), and this is what says the
/// field-level one is deliberate rather than residual.
#[test]
fn a_shadow_group_missing_one_field_fills_it_from_the_bare_style() {
    use harmonigraph_scene::{ShadowKernel, ShadowStyle};

    let bare = ShadowStyle::default();
    // Every held field differs from the bare fallback. Width and depth also
    // differ from the group defaults, distinguishing the two fallback sources;
    // falloff deliberately shares the same early decay in both defaults.
    let held = ShadowStyle {
        kernel: ShadowKernel::Gaussian,
        width: 0.55,
        spread: 0.17,
        depth: 0.66,
        falloff: 1.2,
    };

    let mut state = fresh();
    // A witness outside the section, so "the blob survived" is distinguishable
    // from "it sank and everything reverted".
    state.picture.appearance.camera.yaw = 1.23;
    state.picture.appearance.view.shadow.lattice_geometry = held;
    let saved = state.save_persist();

    let whole = ron::to_string(&held).expect("a shadow style serializes");
    let pairs = top_level_pairs(&whole);
    assert_eq!(pairs.len(), 5, "the probe must see the whole style, got {pairs:?}");

    for (key, _) in &pairs {
        let want = match key.as_str() {
            "width" => ShadowStyle { width: bare.width, ..held },
            "spread" => ShadowStyle { spread: bare.spread, ..held },
            "depth" => ShadowStyle { depth: bare.depth, ..held },
            "falloff" => ShadowStyle { falloff: bare.falloff, ..held },
            "kernel" => ShadowStyle { kernel: bare.kernel, ..held },
            other => panic!("a shadow style grew a {other:?} field this sweep does not name"),
        };
        assert_ne!(want, held, "dropping {key:?} must exercise a fallback");

        let kept: Vec<&str> =
            pairs.iter().filter(|(k, _)| k != key).map(|(_, text)| text.as_str()).collect();
        let short = format!("({})", kept.join(","));
        let without = replace_pair(&saved, "lattice_geometry", &whole, &short);
        assert_ne!(without, saved, "the group's {key:?} must be in the blob to drop");

        let mut restored = fresh();
        assert!(
            restored.load_persist(&without),
            "dropping the group's {key:?} sank the whole document instead of costing itself",
        );
        assert_eq!(
            restored.picture.appearance.camera.yaw, 1.23,
            "dropping the group's {key:?} cost the camera too",
        );
        assert_eq!(
            restored.picture.appearance.view.shadow.lattice_geometry, want,
            "dropping the group's {key:?} did not fill it from the bare style, \
             or cost the fields beside it",
        );
    }
}

/// A missing shape inside the glow curve costs that field alone. The curve is
/// a persisted struct nested inside `ViewConfig`, so the view-level sweep
/// above only proves the whole curve has a fallback; it cannot see whether its
/// field accidentally became required.
#[test]
fn a_glow_curve_missing_shape_uses_the_fresh_shape() {
    use harmonigraph_scene::GlowCurve;

    let curve = GlowCurve { shape: -3.0 };
    let fresh = GlowCurve::default();
    let full = ron::to_string(&curve).expect("a glow curve serializes");
    let pairs = top_level_pairs(&full);
    assert_eq!(pairs.len(), 1, "the probe must see the curve's one shape: {full}");
    let loaded = ron::from_str::<GlowCurve>("()")
        .unwrap_or_else(|e| panic!("dropping shape sank the glow curve: {e}"));
    assert_eq!(loaded, fresh);
}

/// A key the struct no longer has costs that key alone: the rest of the view
/// loads, at the values the blob holds.
///
/// The other half of the sweep above, and the half that decides what a RETIRED
/// field costs. A saved project still carries every key it was written with —
/// `glow_feather` and `glow_meld` among them — and nothing in the tree reads
/// them; what must not happen is the parse failing and taking the whole view
/// with it, which is what `deny_unknown_fields` anywhere on this struct would
/// buy. Asked of an invented key rather than of a retired one, because the
/// property is about the DOOR and a named ex-field would make it a test of
/// that field's removal.
#[test]
fn a_view_carrying_a_key_the_struct_does_not_have_loads_intact() {
    // A reach that is not the fresh one, so the test can tell a blob that
    // loaded from a blob that fell back to `Default` whole.
    let fresh = harmonigraph_scene::ViewConfig { glow_reach: 2.88, ..Default::default() };
    let full = ron::to_string(&fresh).expect("a view serializes");
    let with_extra = full.replacen('(', "(a_key_no_field_answers_to:1.0,", 1);
    let loaded = ron::from_str::<harmonigraph_scene::ViewConfig>(&with_extra)
        .expect("an unknown key must not sink the view");
    assert_eq!(
        ron::to_string(&loaded).expect("a view serializes"),
        full,
        "an unknown key changed what the rest of the blob loaded as",
    );
}

#[test]
fn atmosphere_keys_default_individually_and_normalize_on_load() {
    use harmonigraph_scene::AtmosphereSettings;
    let mut state = fresh();
    state.picture.appearance.camera.yaw = 1.23;
    state.picture.appearance.view.atmosphere = AtmosphereSettings {
        texture: harmonigraph_scene::LatticeTexture::None,
        material_style: harmonigraph_scene::LatticeMaterial::Mosaic,
        material_amount: 0.63,
        material_settings: harmonigraph_scene::MaterialSettings {
            scale_size: 1.7,
            ..Default::default()
        },
        material_speed: 0.8,

        texture_depth: 0.45,
        breath_speed: 2.2,
        ..Default::default()
    };
    let saved = state.save_persist();
    let full = ron::to_string(&state.picture.appearance.view.atmosphere).unwrap();
    let retired = format!("(wide_strength:0.9,wide_spread:6.0,{}", &full[1..]);
    let old = replace_pair(&saved, "atmosphere", &full, &retired);
    let mut restored = fresh();
    assert!(restored.load_persist(&old), "retired wide-glow keys sank the document");
    assert_eq!(restored.picture.appearance.camera.yaw, 1.23);
    assert_eq!(
        restored.picture.appearance.view.atmosphere,
        state.picture.appearance.view.atmosphere
    );

    let defaults = ron::to_string(&AtmosphereSettings::default()).unwrap();
    let pairs = top_level_pairs(&full);
    let fresh_pairs: std::collections::HashMap<_, _> =
        top_level_pairs(&defaults).into_iter().collect();
    for (key, _) in &pairs {
        let kept: Vec<_> =
            pairs.iter().filter(|(k, _)| k != key).map(|(_, text)| text.as_str()).collect();
        let without = replace_pair(&saved, "atmosphere", &full, &format!("({})", kept.join(",")));
        assert_ne!(saved, without, "the omitted key must be in the saved blob");
        let mut restored = fresh();
        assert!(restored.load_persist(&without), "omitting {key} sank the document");
        assert_eq!(restored.picture.appearance.camera.yaw, 1.23);
        let loaded = ron::to_string(&restored.picture.appearance.view.atmosphere).unwrap();
        let loaded: std::collections::HashMap<_, _> =
            top_level_pairs(&loaded).into_iter().collect();
        for (other, text) in &pairs {
            let expected = if other == key { &fresh_pairs[other] } else { text };
            assert_eq!(&loaded[other], expected, "omitting {key} changed {other}");
        }
    }
    // Retired keys are ignored rather than parsed as the new stage enums.
    let old = replace_pair(&saved, "atmosphere", &full,
        "(enabled:false,material:Watercolor,nebula_depth:0.9,nebula_scale:2.0,nebula_speed:3.0,breath_amount:0.6,breath_speed:1.5,source_roughness:0.7)");
    let mut restored = fresh();
    assert!(restored.load_persist(&old));
    assert_eq!(restored.picture.appearance.camera.yaw, 1.23);
    assert_eq!(
        restored.picture.appearance.view.atmosphere,
        AtmosphereSettings { breath_amount: 0.6, breath_speed: 1.5, ..Default::default() }
    );
    state.picture.appearance.view.atmosphere.material_amount = 7.0;
    state.picture.appearance.view.atmosphere.material_settings.scale_size = f32::NAN;
    state.picture.appearance.view.atmosphere.material_speed = -2.0;
    state.picture.appearance.view.atmosphere.texture_depth = f32::NAN;
    state.picture.appearance.view.atmosphere.breath_amount = 7.0;
    let restored = crate::AppearanceDocument::parse(&state.picture.appearance.serialize()).unwrap();
    assert_eq!(restored.view.atmosphere.texture_depth, AtmosphereSettings::default().texture_depth);
    assert_eq!(restored.view.atmosphere.breath_amount, 1.0);
    assert_eq!(restored.view.atmosphere.material_amount, 1.0);
    assert_eq!(
        restored.view.atmosphere.material_settings.scale_size,
        AtmosphereSettings::default().material_settings.scale_size
    );
    assert_eq!(restored.view.atmosphere.material_speed, 0.0);
}

#[test]
fn star_rendering_controls_default_old_saves_and_roundtrip() {
    use harmonigraph_scene::{SpectralAtmosphere, StarHaloProfile};
    let old: SpectralAtmosphere =
        ron::from_str("(star_halo_resolution:0.625,pitch_softness:12.0)").unwrap();
    assert_eq!(old.stars.star_halo_profile, StarHaloProfile::Medium);
    assert_eq!(old.stars, harmonigraph_scene::StarSettings::default());
    assert_eq!(old.pitch_softness, 12.0);
    assert_eq!(old.stars.star_far_fill, 0.0);

    let partial: harmonigraph_scene::StarSettings = ron::from_str("(star_jitter:0.23)").unwrap();
    assert_eq!(
        partial,
        harmonigraph_scene::StarSettings { star_jitter: 0.23, ..Default::default() }
    );
    for profile in [
        StarHaloProfile::P3,
        StarHaloProfile::Medium,
        StarHaloProfile::Low,
        StarHaloProfile::Uniform,
    ] {
        let mut state = fresh();
        state.picture.appearance.view.atmosphere.stars.star_jitter = 0.37;
        state.picture.appearance.view.atmosphere.stars.star_halo_profile = profile;
        state.picture.appearance.view.atmosphere.material_style =
            harmonigraph_scene::LatticeMaterial::Stars;
        state.picture.appearance.spectrum.atmosphere.stars.star_halo_profile = profile;
        state.picture.appearance.spectrum.atmosphere.stars.star_halo_resolution = 0.625;
        state.picture.appearance.spectrum.atmosphere.stars.star_far_fill = 0.42;
        let saved = state.save_persist();
        let mut editor = fresh();
        assert!(editor.load_persist(&saved));
        assert_eq!(editor.picture.appearance.view.atmosphere.stars.star_jitter, 0.37);
        assert_eq!(editor.picture.appearance.view.atmosphere.stars.star_halo_profile, profile);
        assert_eq!(
            editor.picture.appearance.view.atmosphere.material_style,
            harmonigraph_scene::LatticeMaterial::Stars
        );
        assert_eq!(editor.picture.appearance.spectrum.atmosphere.stars.star_halo_profile, profile);
        assert_eq!(editor.picture.appearance.spectrum.atmosphere.stars.star_halo_resolution, 0.625);
        let offline =
            crate::AppearanceDocument::parse(&state.picture.appearance.serialize()).unwrap();
        assert_eq!(offline.spectrum.atmosphere.stars.star_halo_profile, profile);
        assert_eq!(offline.view.atmosphere.stars.star_halo_profile, profile);
        assert_eq!(offline.spectrum.atmosphere.stars.star_halo_resolution, 0.625);
        assert_eq!(offline.spectrum.atmosphere.stars.star_far_fill, 0.42);
        assert_eq!(editor.picture.appearance.spectrum.atmosphere.stars.star_far_fill, 0.42);
    }
}

#[test]
fn spectral_atmosphere_defaults_missing_controls_and_repairs_loaded_values() {
    use harmonigraph_scene::SpectralAtmosphere;
    let partial: SpectralAtmosphere = ron::from_str(
        "(diffusion:0.23, blur_mix:0.6, enabled:false, glow:0.4, spread:2.0, texture:0.8)",
    )
    .unwrap();
    assert_eq!(partial, SpectralAtmosphere { spread: 2.0, ..Default::default() });
    let mut state = fresh();
    state.picture.appearance.spectrum.atmosphere = SpectralAtmosphere {
        stars: harmonigraph_scene::StarSettings {
            star_jitter: 2.0,
            star_far_fill: 2.0,
            star_halo_resolution: 0.1,
            ..Default::default()
        },
        pitch_softness: f32::NAN,
        contours: 20.0,
        cloud_direction: 725.0,

        ..Default::default()
    };
    state.picture.appearance.camera.yaw = 1.23;
    let saved = state.save_persist();
    let mut editor = fresh();
    assert!(editor.load_persist(&saved));
    let offline = crate::AppearanceDocument::parse(&state.picture.appearance.serialize()).unwrap();
    let expected = SpectralAtmosphere {
        stars: harmonigraph_scene::StarSettings {
            star_jitter: 1.0,
            star_far_fill: 1.0,
            star_halo_resolution: 0.25,
            ..Default::default()
        },
        contours: 16.0,
        cloud_direction: 5.0,

        ..Default::default()
    };
    assert_eq!(editor.picture.appearance.spectrum.atmosphere, expected);
    assert_eq!(offline.spectrum.atmosphere, expected);
    assert_eq!(editor.picture.appearance.camera.yaw, 1.23);
}

/// A settings section folded in the editor stays folded across the window
/// closing and reopening.
///
/// The plugin builds a brand-new egui `Context` for every window it opens, so a
/// fold kept in egui memory springs open again with the window — the same class
/// of trap as the stale `TextureHandle`
/// (`PictureState::release_context_resources`). The folds live in `UiPersist`
/// instead, and this holds the whole path: a REAL click on a section header in
/// the dock, `save_persist`, then a fresh `Context` and `load_persist`. Both
/// halves are load-bearing — writing the field by hand would pass with the
/// click never wired to it, and asserting inside one `Context` would pass with
/// the state memory-backed, which is the live bug this exists to catch.
#[test]
fn a_folded_section_survives_an_editor_reopen() {
    use super::harness::{press, DockHarness};

    let drawn = |out: &egui::FullOutput, leaf: egui::Rect, needle: &str| {
        out.shapes.iter().any(|cs| match &cs.shape {
            egui::Shape::Text(t) => t.galley.text() == needle && leaf.contains(t.pos),
            _ => false,
        })
    };

    let mut state = fresh();
    state.workspace.layout.select(panes::Tab::AnalyzerSettings);
    let mut window = DockHarness::new();
    window.settle(&mut state);
    let leaf = state.workspace.layout_runtime.rects[workspace::Section::Settings as usize];
    let out = window.frame(&mut state, vec![]);
    assert!(drawn(&out, leaf, "Softness"), "the section opens unfolded");

    // The Spectrogram heading, found where it was painted and clicked for real.
    let header = out
        .shapes
        .iter()
        .find_map(|cs| match &cs.shape {
            egui::Shape::Text(t) if t.galley.text() == "SPECTROGRAM" && leaf.contains(t.pos) => {
                Some(egui::Rect::from_min_size(t.pos, t.galley.size()).center())
            }
            _ => None,
        })
        .expect("the Analyzer page drew no Spectrogram heading");
    window.frame(&mut state, vec![egui::Event::PointerMoved(header)]);
    window.frame(&mut state, vec![egui::Event::PointerMoved(header), press(header, true)]);
    window.frame(&mut state, vec![press(header, false)]);
    let out = window.frame(&mut state, vec![]);
    assert!(
        state.workspace.interaction.folded_sections.contains("Analyzer/Spectrogram"),
        "the click did not reach the persisted field: {:?}",
        state.workspace.interaction.folded_sections,
    );
    assert!(!drawn(&out, leaf, "Softness"), "the click did not fold the section");
    let saved = state.save_persist();

    // The window closes and reopens: a FRESH `Context`, and the state the
    // host hands back through `load_persist`.
    let mut reopened = fresh();
    assert!(reopened.load_persist(&saved), "the blob this build saved must load");
    let mut fresh_window = DockHarness::new();
    fresh_window.settle(&mut reopened);
    let out = fresh_window.frame(&mut reopened, vec![]);
    let leaf = reopened.workspace.layout_runtime.rects[workspace::Section::Settings as usize];
    assert!(drawn(&out, leaf, "SPECTROGRAM"), "the folded section keeps its heading");
    assert!(
        !drawn(&out, leaf, "Softness"),
        "the fold sprang open across the reopen — is its state in egui memory?",
    );
    // A fold is per section: the one below it is still open.
    assert!(drawn(&out, leaf, "Ribbon width"), "folding one section folded another");
}

/// Folding View folds View alone. The analysis sections once drew inside its
/// body, so they vanished with it. The two sections over it are folded too,
/// so the ones under it are inside the window to be seen.
#[test]
fn folding_the_analyzer_view_leaves_the_sections_below_it() {
    let mut state = fresh();
    state.workspace.layout.select(panes::Tab::AnalyzerSettings);
    for section in ["Analyzer/Spectrogram", "Analyzer/MIDI ribbons", "Analyzer/View"] {
        state.workspace.interaction.folded_sections.insert(section.to_owned());
    }
    let mut window = super::harness::DockHarness::new();
    window.settle(&mut state);
    let leaf = state.workspace.layout_runtime.rects[workspace::Section::Settings as usize];
    let out = window.frame(&mut state, vec![]);
    let drawn = |needle: &str| {
        out.shapes.iter().any(|cs| match &cs.shape {
            egui::Shape::Text(t) => t.galley.text() == needle && leaf.contains(t.pos),
            _ => false,
        })
    };
    assert!(!drawn("Spectrum outline intensity"), "the View section did not fold");
    for name in ["ANALYSIS", "Level mapping", "Live response"] {
        assert!(drawn(name), "folding View took {name} with it");
    }
}

/// Split a serialized struct into its top-level `key:value` pairs, as
/// `(key, whole pair)`. Depth-aware, so `pitch_gradient:(...)` stays one pair
/// rather than splitting on the commas inside it — which is equally what lets
/// a whole persist section (`render:(...)`, `dock:(...)`) be dropped or
/// counted as one.
fn top_level_pairs(blob: &str) -> Vec<(String, String)> {
    let trimmed = blob.trim();
    // Exactly ONE enclosing paren off each end. Trimming every one of them takes
    // the last field's own closer along with the container's wherever that field
    // is itself a struct — `(zoom:2.75,look:(0.4,-0.6))` comes back with `look`
    // one paren short, which reads as a pair and rebuilds into a blob that will
    // not parse.
    let inner = trimmed.strip_prefix('(').and_then(|s| s.strip_suffix(')')).unwrap_or(trimmed);
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

/// Replace one `key:value` pair in a serialized blob. RON has no trailing
/// comma on a struct's last field, so `key` closing its struct (`)`) needs a
/// different pattern than one with a sibling after it (`,`) — tried in that
/// order, comma first since most fields have one.
fn replace_pair(blob: &str, key: &str, was: &str, value: &str) -> String {
    let with_comma = blob.replacen(&format!("{key}:{was},"), &format!("{key}:{value},"), 1);
    if with_comma != blob {
        return with_comma;
    }
    blob.replacen(&format!("{key}:{was})"), &format!("{key}:{value})"), 1)
}

/// A hand-edited camera comes back one `view_proj` can draw, on the same
/// footing as the pitch range and the soft-edge pair above:
/// `screen_scale` already guards `distance` locally for the
/// font-size math it does, but nothing stood between a hand-edited blob and
/// `ortho`'s `tan`, or `eye`'s trig, which read the fields directly on every
/// frame the camera is drawn. A non-finite value and a finite one past what
/// the field's own bar can reach are swept together: both repair to the same
/// place, since a NaN and an out-of-range value fall back through the same
/// `finite_or(..).clamp(..)`.
/// (key, bad value, hint, the range a repaired value must fall in — `None`
/// for a field with no natural bound).
type CameraCase = (&'static str, &'static str, &'static str, Option<(f32, f32)>);

#[test]
fn a_blob_naming_a_nonsense_camera_opens_on_what_it_can_reach() {
    let cases: [CameraCase; 9] = [
        ("yaw", "NaN", "a NaN yaw", None),
        ("pitch", "NaN", "a NaN pitch", Some((-Camera::PITCH_LIMIT, Camera::PITCH_LIMIT))),
        (
            "pitch",
            "10.0",
            "a pitch past `orbit`'s limit",
            Some((-Camera::PITCH_LIMIT, Camera::PITCH_LIMIT)),
        ),
        ("distance", "NaN", "a NaN distance", Some((Camera::MIN_DISTANCE, Camera::MAX_DISTANCE))),
        (
            "distance",
            "999.0",
            "a distance past the zoom range",
            Some((Camera::MIN_DISTANCE, Camera::MAX_DISTANCE)),
        ),
        ("cabinet_angle", "NaN", "a NaN cabinet angle", Some((0.0, std::f32::consts::FRAC_PI_2))),
        (
            "cabinet_angle",
            "10.0",
            "a cabinet angle past its bar's 0..=90 degrees",
            Some((0.0, std::f32::consts::FRAC_PI_2)),
        ),
        ("cabinet_scale", "inf", "an infinite cabinet scale", Some((0.1, 1.0))),
        ("cabinet_scale", "5.0", "a cabinet scale past its bar", Some((0.1, 1.0))),
    ];
    for (key, value, hint, range) in cases {
        let mut state = fresh();
        state.picture.appearance.view.max_sevens = 3;
        let saved = state.save_persist();
        let was = match key {
            "yaw" => state.picture.appearance.camera.yaw,
            "pitch" => state.picture.appearance.camera.pitch,
            "distance" => state.picture.appearance.camera.distance,
            "cabinet_angle" => state.picture.appearance.camera.cabinet_angle,
            _ => state.picture.appearance.camera.cabinet_scale,
        };
        let edited = replace_pair(&saved, key, &format!("{was:?}"), value);
        assert_ne!(edited, saved, "{hint}: `{key}` is not in the blob to edit");

        let mut restored = fresh();
        restored.load_persist(&edited);
        let got = match key {
            "yaw" => restored.picture.appearance.camera.yaw,
            "pitch" => restored.picture.appearance.camera.pitch,
            "distance" => restored.picture.appearance.camera.distance,
            "cabinet_angle" => restored.picture.appearance.camera.cabinet_angle,
            _ => restored.picture.appearance.camera.cabinet_scale,
        };
        assert!(got.is_finite(), "{hint}: `{key}` opened at {got}");
        if let Some((lo, hi)) = range {
            assert!(got >= lo && got <= hi, "{hint}: `{key}` opened at {got}, outside {lo}..={hi}");
        }
        assert_eq!(
            restored.picture.appearance.view.max_sevens, 3,
            "{hint}: the rest of the blob still restores"
        );
    }
}

/// A SAVED ANGLE carries the same two fields the camera does, and reaches the
/// camera by plain assignment — the Display/View preset buttons write
/// `camera.yaw`/`camera.pitch` straight through, so a preset is neither
/// `orbit` (which clamps every drag) nor `Camera::sanitize` (which clamps the
/// load). It is the one remaining door into those two fields that fits nothing.
///
/// Both halves matter and fail differently. A NaN yaw takes the whole lattice
/// out: `eye()` returns an all-NaN direction, `look_at_rh` an all-NaN view
/// matrix, and the pane draws bare background with no way back, since `orbit`'s
/// `yaw -= delta` leaves NaN NaN. An out-of-range pitch is the quieter half and
/// the one the repo's rule names directly — the camera sits past where any
/// control can put it while the "Camera pitch" bar reads the raw number with
/// its fill pinned at the end of a range that does not contain it.
#[test]
fn a_saved_angle_lands_where_the_camera_controls_could_have_put_it() {
    for (hint, yaw, pitch) in
        [("a NaN yaw", "NaN", "0.0"), ("a pitch past `orbit`'s limit", "0.0", "10.0")]
    {
        let mut state = fresh();
        state.picture.appearance.view.max_sevens = 3;
        state.workspace.interaction.camera_presets.push(crate::CameraPreset {
            name: "reading".to_string(),
            yaw: 0.25,
            pitch: 0.5,
        });
        let saved = state.save_persist();
        let edited = saved.replacen("yaw:0.25,pitch:0.5", &format!("yaw:{yaw},pitch:{pitch}"), 1);
        assert_ne!(edited, saved, "{hint}: the preset is not in the blob to edit");

        let mut restored = fresh();
        restored.load_persist(&edited);
        assert_eq!(
            restored.picture.appearance.view.max_sevens, 3,
            "{hint}: the rest of the blob still restores"
        );
        let preset = &restored.workspace.interaction.camera_presets[0];

        // Applied exactly as the preset button applies it.
        let mut camera = restored.picture.appearance.camera;
        camera.yaw = preset.yaw;
        camera.pitch = preset.pitch;
        assert!(camera.yaw.is_finite(), "{hint}: the camera's yaw became {}", camera.yaw);
        assert!(
            camera.pitch >= -Camera::PITCH_LIMIT && camera.pitch <= Camera::PITCH_LIMIT,
            "{hint}: the camera's pitch became {}, outside {}..={}",
            camera.pitch,
            -Camera::PITCH_LIMIT,
            Camera::PITCH_LIMIT,
        );
        assert!(camera.eye().is_finite(), "{hint}: `eye()` returned {:?}", camera.eye());
    }
}

/// The target has no bar and no natural bound — the camera can look
/// anywhere — so unlike its five neighbours above it is repaired rather than
/// clamped, and as a whole vector: `eye()` reads all three components
/// together, and one bad component would otherwise NaN the other two
/// through it.
#[test]
fn a_blob_naming_a_nonsense_camera_target_opens_on_a_drawable_one() {
    let mut state = fresh();
    state.picture.appearance.view.max_sevens = 3;
    let saved = state.save_persist();
    let edited = saved.replace("target:(0.0,0.0,0.0),", "target:(NaN,0.0,0.0),");
    assert_ne!(edited, saved, "`target` is not in the blob to edit");

    let mut restored = fresh();
    restored.load_persist(&edited);
    assert!(
        restored.picture.appearance.camera.target.is_finite(),
        "a NaN component opened at {:?}",
        restored.picture.appearance.camera.target,
    );
    assert_eq!(
        restored.picture.appearance.view.max_sevens, 3,
        "the rest of the blob still restores"
    );
}

/// The Video pane's split dial: `split` feeds `Layout::split`, whose own
/// clamp cannot repair a NaN (it loses every comparison a clamp makes), only
/// hold a finite value inside a literal range.
#[test]
fn a_blob_naming_a_nonsense_render_config_opens_on_what_it_can_reach() {
    let cases: [(&str, &str, (f32, f32)); 2] =
        [("NaN", "a NaN split", (0.05, 0.95)), ("inf", "an infinite split", (0.05, 0.95))];
    for (value, hint, (lo, hi)) in cases {
        let mut state = fresh();
        state.picture.appearance.view.max_sevens = 3;
        let saved = state.save_persist();
        let was = state.picture.appearance.render.frame.split;
        let edited = replace_pair(&saved, "split", &format!("{was:?}"), value);
        assert_ne!(edited, saved, "{hint}: `split` is not in the blob to edit");

        let mut restored = fresh();
        restored.load_persist(&edited);
        let got = restored.picture.appearance.render.frame.split;
        assert!(got.is_finite(), "{hint}: `split` opened at {got}");
        assert!(got >= lo && got <= hi, "{hint}: `split` opened at {got}, outside {lo}..={hi}");
        assert_eq!(
            restored.picture.appearance.view.max_sevens, 3,
            "{hint}: the rest of the blob still restores"
        );
    }
}

#[test]
fn animation_controls_round_trip_and_a_missing_one_loads_fresh() {
    for order in harmonigraph_scene::AnimationOrder::ALL {
        let mut state = fresh();
        state.picture.appearance.view.note_animation = harmonigraph_scene::NoteAnimationConfig {
            order,
            stagger_spread: 0.63,
            radial_start: -0.5,
        };
        let mut restored = fresh();
        assert!(restored.load_persist(&state.save_persist()));
        assert_eq!(
            restored.picture.appearance.view.note_animation,
            state.picture.appearance.view.note_animation
        );
    }
    let mut state = fresh();
    state.picture.appearance.view.label_scale = 0.7;
    state.picture.appearance.view.note_animation.order =
        harmonigraph_scene::AnimationOrder::Circular;
    state.picture.appearance.view.note_animation.stagger_spread = 0.63;
    let old = state.save_persist();
    let missing_spread = old.replace("stagger_spread:0.63,", "");
    assert_ne!(old, missing_spread);
    assert!(state.load_persist(&missing_spread));
    assert_eq!(
        state.picture.appearance.view.note_animation.stagger_spread,
        harmonigraph_scene::NoteAnimationConfig::default().stagger_spread
    );
    assert_eq!(state.picture.appearance.view.label_scale, 0.7);
    assert_eq!(
        state.picture.appearance.view.note_animation.order,
        harmonigraph_scene::AnimationOrder::Circular
    );
    let saved = state.save_persist().replace("note_animation:", "retired_animation_config:");
    assert!(!saved.contains("note_animation:"));
    assert!(state.load_persist(&saved));
    assert_eq!(state.picture.appearance.view.note_animation, Default::default());
    assert_eq!(state.picture.appearance.view.label_scale, 0.7);
}

/// Retired dock fields are ignored rather than taking appearance with them.
#[test]
fn an_old_dock_is_replaced_without_losing_appearance() {
    let mut state = fresh();
    state.picture.appearance.camera.yaw = 1.23;
    let saved = state.save_persist();
    let kept: Vec<_> = top_level_pairs(&saved)
        .into_iter()
        .filter(|(key, _)| key != "layout")
        .map(|(_, text)| text)
        .collect();
    let old = format!("({},dock:(tabs:[Notes,Console]),folds:[])", kept.join(","));
    let mut restored = fresh();
    assert!(restored.load_persist(&old));
    assert_eq!(restored.picture.appearance.camera.yaw, 1.23);
    assert_eq!(restored.workspace.layout.position, workspace::Position::Right);
    assert_eq!(restored.workspace.layout.folded, [false; 3]);
}

#[test]
fn material_settings_are_independent_and_missing_nested_keys_default() {
    use harmonigraph_scene::MaterialSettings;
    let mut state = fresh();
    state.picture.appearance.view.atmosphere.material_style =
        harmonigraph_scene::LatticeMaterial::VelvetScales;
    state.picture.appearance.spectrum.atmosphere.cloud_style =
        harmonigraph_scene::CloudStyle::VelvetScales;
    let lattice = &mut state.picture.appearance.view.atmosphere.material_settings;
    lattice.wash_fuzz = 0.23;
    lattice.scale_variety = 0.81;
    lattice.velvet_edge = 0.51;
    lattice.velvet_irregularity = 0.43;
    lattice.velvet_shape = 0.72;
    let spectral = &mut state.picture.appearance.spectrum.atmosphere.material_settings;
    spectral.wash_fuzz = 0.72;
    spectral.scale_variety = 0.19;
    spectral.velvet_edge = 0.18;
    spectral.velvet_irregularity = 0.93;
    spectral.velvet_shape = 0.29;
    let mut restored = fresh();
    assert!(restored.load_persist(&state.save_persist()));
    assert_eq!(
        restored.picture.appearance.view.atmosphere.material_settings,
        state.picture.appearance.view.atmosphere.material_settings
    );
    assert_eq!(
        restored.picture.appearance.spectrum.atmosphere.material_settings,
        state.picture.appearance.spectrum.atmosphere.material_settings
    );
    let partial: MaterialSettings = ron::from_str("(wash_fuzz:0.23)").unwrap();
    assert_eq!(partial, MaterialSettings { wash_fuzz: 0.23, ..Default::default() });
}
