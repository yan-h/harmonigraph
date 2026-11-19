//! What survives a session: round-trips through [`crate::state::UiPersist`], the version
//! floor under it, and what a blob costs when a key it carries — or one it is
//! missing — is not the shape this build expects.

use super::harness::collapsed;
use super::probe::{fresh, Defaults};
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
    appearance.view.note_animation.radial_start = -0.5;
    appearance.view.note_animation.stagger_spread = 0.63;
    appearance.view.atmosphere.texture_depth = 0.0;
    appearance.spectrum.orientation = crate::SpectralOrientation::Left;
    appearance.spectrum.roll_thickness = 1.75;
    appearance.spectrum.low_midi = 40.5;
    appearance.render.short_edge = 2160;
    appearance.render.frame.split = 0.37;
    appearance.render.spectrogram = crate::SpectrogramRender::Scrolling;
    state.workspace.interaction.prefs.ui_scale = 1.25;
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

/// One retired key per SHAPE of value a retirement can leave in saved
/// projects, as `(a live key in the same struct, the retired pair as an older
/// build wrote it, whether that struct is part of the appearance)`: the
/// retired pair is spliced back in ahead of the live key. The last is `false`
/// for editor-only state, which only the editor door reads.
///
/// The shape is what is tested, not the name. A number is skipped by any
/// parser; a bare identifier (`Fibres`) is an enum token with no type left to
/// parse it into, and a string, `NaN`, an `Option` and a nested struct are
/// tokens of their own. Retiring a FIELD costs nothing for every one of them,
/// where retiring a VARIANT costs the key that names it — that is
/// [`a_document_naming_any_one_unknown_variant_costs_that_key_alone`].
///
/// So retiring a key adds no row (#1677 area 5 Q1): serde ignores it the way
/// it ignores the row of its shape. A row is owed only by a shape none of
/// these has.
const RETIRED_KEYS: &[(&str, &str, bool)] = &[
    // A number.
    ("pitch_softness", "spread:0.75,", true),
    // A non-finite number.
    ("label_scale", "spacing:NaN,", true),
    // A flag.
    ("stagger_spread", "lit_first:false,", true),
    // An enum token, its type gone with the key.
    ("material_style", "material:Fibres,", true),
    // An `Option`, empty and filled; the filled one through the editor door alone.
    ("texture_depth", "texture:None,", true),
    ("frame_rate", "fps_cap:Some(90.0),", false),
    // A string.
    ("short_edge", "renderer_path:\"/old/renderer\",", true),
    // A nested struct.
    ("spectrum", "spiral:(zoom:3.0,look:(0.4,-0.6)),", true),
    // A list of enum tokens, at the editor document's own top level.
    ("prefs", "folds:[Notes,Console],", false),
    // A key that moved to another struct under the same name: one left inside
    // a view must not reach the preference.
    ("render_scale", "show_perf:true,show_perf_detail:true,", true),
];

/// A retired key costs nothing: through both doors — the editor's
/// `install_document`, and, for a key inside the appearance, the
/// `AppearanceDocument::parse` that take replay and explicit export share —
/// the rest of the document loads at the values it holds, and the key is gone
/// on the next save.
///
/// `AppearanceDocument::parse` takes the whole appearance or nothing, so a key
/// that failed to parse there would not degrade: it would refuse the take's
/// export. Every retirement rides on serde ignoring what it has no field for,
/// and this is where that is held.
#[test]
fn a_retired_key_is_ignored_and_the_rest_survives() {
    let state = dialled();
    let saved = state.save_persist();
    let appearance = state.picture.appearance.serialize();
    for &(anchor, retired, in_appearance) in RETIRED_KEYS {
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
        assert!(restored.install_document(&splice(&saved)), "{retired} sank the editor blob");
        assert_eq!(restored.save_persist(), saved, "{retired} changed what the editor loaded");

        assert_eq!(!pair_starts(&appearance, anchor).is_empty(), in_appearance, "{anchor}: doors");
        if in_appearance {
            let parsed = AppearanceDocument::parse(&splice(&appearance))
                .unwrap_or_else(|e| panic!("{retired} sank the appearance: {e}"));
            assert_eq!(
                parsed.serialize(),
                appearance,
                "{retired} changed what the appearance loaded"
            );
        }
    }
}

/// [`dialled`], plus a camera preset, a look stored in slot 2 and the
/// geometry an unfolded Analyzer frame leaves: a document whose sweeps reach
/// the structs inside a list, a tuple and a `Some`.
fn filled() -> SharedState {
    use crate::appearance_edit::{AppearanceEditor, Look, Slots};

    let mut state = dialled();
    state.workspace.interaction.camera_presets.push(CameraPreset {
        name: "reading".into(),
        yaw: 0.7,
        pitch: 0.2,
    });
    let mut stored = AppearanceDocument::default();
    stored.spectrum.attack = 0.2;
    let look = ron::to_string(&Look::capture(&stored)).expect("a look serializes");
    let slots: Slots = ron::from_str(&format!("(stored:(None,Some({look}),None,None),active:0)"))
        .expect("a slot holding a look parses");
    state.workspace.interaction.appearance_editor = AppearanceEditor::restore(slots);
    // What a drawn, unfolded Analyzer records, written as its save has it:
    // the geometry is private to the pane that draws it.
    state.workspace.interaction.analyzer_regions =
        ron::from_str("(geometry:Some((depths:(300.0,200.0),dial:0.5,vertical:false)))")
            .expect("an Analyzer's regions parse");
    state
}

/// Every enum token a filled document holds, swapped in turn for a variant no
/// build has, costs the editor door the key that holds it and nothing else
/// (#1757 Q4): the key opens at its default, the Console opens on a line
/// naming its path, the rest loads as the same document with that pair cut
/// out would, and a save stores what loaded. The take and export door refuses
/// an appearance naming it whole (Q5).
///
/// So retiring a variant owes no row here: the sweep reaches every variant a
/// save writes.
#[test]
fn a_document_naming_any_one_unknown_variant_costs_that_key_alone() {
    const UNKNOWN: &str = "NoSuchVariant";
    let saved = filled().save_persist();
    let appearance = filled().picture.appearance.serialize();
    let pairs = nested_pairs(&saved);
    let (_, nested) = pairs.iter().find(|(path, _)| path == "appearance").expect("an appearance");
    let nested = nested.start + "appearance:".len()..nested.end;
    assert_eq!(saved[nested.clone()], appearance, "the editor save nests the appearance as is");
    let is_variant = |token: &str| {
        token.starts_with(|c: char| c.is_ascii_uppercase())
            && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !matches!(token, "None" | "NaN")
    };
    let variants: Vec<_> = nested_scalars(&saved)
        .into_iter()
        .filter(|(_, at)| is_variant(&saved[at.clone()]))
        .collect();
    // The walk sees a variant only as a scalar: one carrying a payload,
    // `Name(…)`, it walks straight through. None is persisted today; the day
    // one is, this sweep stops covering it, so it says so here.
    let mut quoted = false;
    let carried: Vec<&str> = saved
        .char_indices()
        .filter(|&(_, c)| {
            quoted ^= c == '"';
            c == '(' && !quoted
        })
        .filter_map(|(at, _)| {
            let name = saved[..at].rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_'));
            name.into_iter().next().filter(|name| !name.is_empty() && *name != "Some")
        })
        .collect();
    assert!(
        carried.is_empty(),
        "variants carrying a payload, which the sweep cannot swap: {carried:?}"
    );
    // The fixture has to reach every kind of place a variant sits, or the
    // sweep reads as coverage of what it never touches.
    for deep in [
        "layout.settings_tab",
        "appearance.camera.projection",
        "appearance.view.atmosphere.material_style",
        "appearance.spectrum.orientation",
        "look_slots.stored[1].spectrum.window",
        "prefs.frame_rate",
    ] {
        assert!(variants.iter().any(|(path, _)| path == deep), "the sweep never reaches {deep}");
    }

    for (path, at) in &variants {
        let swap = |blob: &str, at: std::ops::Range<usize>| {
            format!("{}{UNKNOWN}{}", &blob[..at.start], &blob[at.end..])
        };
        let swapped = swap(&saved, at.clone());
        // The pair whose value holds the token: a tuple's item is its pair's.
        let mut owner = path.as_str();
        while let Some(item) = owner.strip_suffix(']') {
            owner = &item[..item.rfind('[').expect("an index opens")];
        }
        let (_, pair) = pairs.iter().find(|(p, _)| p == owner).expect("a pair holds every token");
        let named: String = owner
            .split('.')
            .map(|key| &key[..key.find('[').unwrap_or(key.len())])
            .collect::<Vec<_>>()
            .join(".");

        let mut restored = fresh();
        assert!(
            restored.install_document(&swapped),
            "{path}: an unknown variant sank the document"
        );
        assert_eq!(
            restored.save_answer().as_ref(),
            restored.save_persist(),
            "{path}: a save does not store what loaded",
        );
        assert!(!collapsed(&restored, panes::Tab::Console), "{path}: the loss is hidden");
        let said = restored.picture.runtime.console.lines().find(|line| line.contains(UNKNOWN));
        assert!(
            said.is_some_and(|line| line.contains(&format!("dropped {named} "))),
            "{path}: the Console does not name {named}: {said:?}",
        );

        let cut = if saved[pair.end..].starts_with(',') {
            pair.start..pair.end + 1
        } else {
            pair.start..pair.end
        };
        let mut without = fresh();
        assert!(without.install_document(&format!("{}{}", &saved[..cut.start], &saved[cut.end..])));
        set_console_collapsed(&mut restored, true);
        set_console_collapsed(&mut without, true);
        assert_eq!(
            restored.save_persist(),
            without.save_persist(),
            "{path}: more than its key was lost, or less",
        );

        if nested.contains(&at.start) {
            let inside = at.start - nested.start..at.end - nested.start;
            assert!(
                AppearanceDocument::parse(&swap(&appearance, inside)).is_err(),
                "{path} parsed"
            );
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
        // Off the default and different per group, on the rule the head of
        // this test states: `groups()` compares whole styles, so a spread left
        // at its default would round-trip by matching the default rather than
        // by being carried.
        group.spread = 0.1 + index as f32 * 0.2;
    }
    // Which nodes are named, and the fresh view keeps the past -- so either
    // other mode is a value a project has to keep, and the one a fresh view
    // would overwrite if it did not.
    state.picture.appearance.view.note_names = NoteNames::All;
    state.workspace.interaction.camera_presets.push(CameraPreset {
        name: "reading".into(),
        yaw: 0.7,
        pitch: 0.2,
    });
    let saved = state.save_persist();

    let mut restored = fresh();
    restored.install_document(&saved);
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
    assert_eq!(restored.workspace.interaction.camera_presets.len(), 1);
    assert_eq!(restored.workspace.interaction.camera_presets[0].name, "reading");
    assert_eq!(restored.workspace.interaction.camera_presets[0].yaw, 0.7);
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
        restored.install_document(&edited);
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
    restored.install_document(&edited);
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
    restored.install_document(&saved);
    assert_eq!(restored.picture.appearance.render.frame.lattice, LatticeSide::Bottom);
    assert_eq!(restored.picture.appearance.render.frame.split, 0.42);
    assert_eq!(restored.picture.appearance.camera.yaw, 1.23, "rest of the blob still restores");
}

#[test]
fn corrupt_persist_is_ignored() {
    let mut state = fresh();
    let default_distance = state.picture.appearance.camera.distance;
    assert!(!state.install_document("not json at all"), "a corrupt blob is not applied");
    assert_eq!(state.picture.appearance.camera.distance, default_distance);
    assert!(!collapsed(&state, panes::Tab::Console), "the explanation is visible");
}

/// A refused blob SAYS SO. A refusal costs the whole document — dock,
/// camera, view, spectrum and render at once — and a project reopening at
/// defaults with nothing written anywhere reads as data loss rather than as a
/// break someone chose. A dropped variant costs only its key
/// ([`a_document_naming_any_one_unknown_variant_costs_that_key_alone`]); these are the version
/// floor's and the appearance version's refusals.
#[test]
fn a_refused_blob_says_why() {
    let saved = fresh().save_persist();
    let stale = saved.replacen(
        &format!("version:{UI_PERSIST_VERSION}"),
        &format!("version:{}", UI_PERSIST_VERSION - 1),
        1,
    );
    let mut older = fresh();
    assert!(!older.install_document(&stale), "a blob below the floor is not applied");
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
    assert!(!restored.install_document(&unsupported));
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

/// A save stores the settings the editor holds (#1757): one installed whole
/// saves back as it came, a refused one leaves the settings, and so the save,
/// as they were, and the empty one — what an editor never opened saves — puts
/// the fresh settings back, which a save then stores, with nothing on the
/// Console. Only an editor that has held nothing saves empty.
#[test]
fn an_install_decides_what_a_save_stores() {
    let saved = dialled().save_persist();
    let mut state = fresh();
    assert_eq!(state.save_answer().as_ref(), "", "a fresh editor saves nothing");

    assert!(state.install_document(&saved));
    assert_eq!(state.save_answer().as_ref(), saved, "a whole install did not save back as it came");

    assert!(!state.install_document("not a blob"));
    set_console_collapsed(&mut state, true);
    assert_eq!(state.save_answer().as_ref(), saved, "a refusal moved more than the Console");

    let lines = state.picture.runtime.console.lines().count();
    assert!(state.install_document(""));
    assert_eq!(
        state.save_answer().as_ref(),
        state.save_persist(),
        "an empty restore into an editor that held settings saves nothing"
    );
    assert_eq!(
        state.save_persist(),
        fresh().save_persist(),
        "the empty document is not the fresh one"
    );
    assert_eq!(
        state.picture.runtime.console.lines().count(),
        lines,
        "the empty document is not news"
    );
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
    restored.install_document(&saved);
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

/// The nested groups whose owner's `Default` deliberately builds them off their
/// own bare default, as path prefixes with that bare default serialized and the
/// reason each: a key missing INSIDE one comes back at the same key of the
/// group's bare default, which is not the value a fresh save holds there. Any
/// other key must come back at the fresh save's value. An entry no dropped key
/// needs any more fails the sweep.
///
/// A camera preset is one too, for another reason: a fresh save holds none, so
/// its keys have no fresh value to come back at but their type's default.
fn nested_defaults() -> [(&'static str, String, &'static str); 11] {
    fn bare<T: Default + serde::Serialize>() -> String {
        ron::to_string(&T::default()).expect("a default serializes")
    }
    use harmonigraph_scene::{Gradient, MaterialSettings, ShadowStyle, StarSettings};
    let shadow =
        "each Shadow group is tuned apart from the bare `ShadowStyle` (its `impl Default`)";
    [
        (
            "layout.below",
            bare::<workspace::Sizes>(),
            "the dock below the lattice has its own sizes; `Sizes` defaults to the right's",
        ),
        (
            "appearance.view.pitch_gradient",
            bare::<Gradient>(),
            "the lattice's pitch gradient is a tuned look",
        ),
        (
            "appearance.spectrum.spectrogram_gradient",
            bare::<Gradient>(),
            "the heatmap's gradient and its bend are tuned",
        ),
        (
            "appearance.view.atmosphere.material_settings",
            bare::<MaterialSettings>(),
            "the lattice glow's captured Watercolor",
        ),
        (
            "appearance.view.atmosphere.stars",
            bare::<StarSettings>(),
            "the lattice's captured stars, four layers of five",
        ),
        (
            "appearance.spectrum.atmosphere.material_settings",
            bare::<MaterialSettings>(),
            "the spectrogram's captured texture",
        ),
        (
            "appearance.spectrum.atmosphere.stars",
            bare::<StarSettings>(),
            "the spectrogram's captured stars",
        ),
        ("appearance.view.shadow.lattice_geometry", bare::<ShadowStyle>(), shadow),
        ("appearance.view.shadow.lattice_text", bare::<ShadowStyle>(), shadow),
        ("appearance.view.shadow.spectral_text", bare::<ShadowStyle>(), shadow),
        ("camera_presets[0]", bare::<CameraPreset>(), "a fresh save holds no preset"),
    ]
}

/// The layer under the missing-key tests above and below: every key of every
/// struct the editor save holds, at any depth, dropped in turn — through
/// `Some`, tuples, lists and enum payloads as well — and the rest parsed with
/// the parse `install_document` runs.
///
/// Those tests each sweep one level through the real doors, `install_document` or
/// `AppearanceDocument::parse`, and compare what loads: the persist sections
/// (`a_persist_blob_missing_any_one_section_keeps_the_rest`) and the
/// appearance's groups (`an_appearance_missing_any_one_group_keeps_the_rest`).
/// This one reaches every struct nested further in, a preset and a stored look
/// included, asked at the parse alone, which is the parse `install_document`'s
/// per-key decode runs, since normalization may rightly move a sibling of the
/// dropped key.
///
/// Each drop must cost that key alone: the rest reserializes at the value
/// saved, and the dropped key comes back at the value a fresh save holds there
/// — or, inside one of [`nested_defaults`], at its group's bare default. A
/// struct without its container-level `#[serde(default)]` fails the parse, and
/// one whose fields carry their own defaults loads the key at 0 or `false`.
/// Exempt from the value check is the editor's `version`, which reads as 0
/// for the floor to refuse.
///
/// The fixture is `dialled`, which moves a few values in the camera, view,
/// spectrum, render and workspace off fresh, plus a camera preset, a stored
/// look and the Analyzer's region geometry, so the walk reaches the structs
/// inside a list, a tuple and a `Some`.
#[test]
fn a_persist_blob_missing_any_one_key_at_any_depth_keeps_the_rest() {
    let saved = filled().save_persist();
    let pairs = nested_pairs(&saved);
    let fresh_save = fresh().save_persist();
    let fresh_pairs: std::collections::HashMap<_, _> = nested_pairs(&fresh_save)
        .into_iter()
        .map(|(path, range)| (path, fresh_save[range].to_string()))
        .collect();
    // The fixture has to reach the nested structs, or the sweep passes over
    // the top level alone and reads as coverage.
    for deep in [
        "appearance.camera.yaw",
        "appearance.view.pitch_gradient.bend.at",
        "appearance.view.intensity.velocity.opacity",
        "appearance.view.atmosphere.stars.star_jitter",
        "appearance.spectrum.atmosphere.material_settings.wash_size",
        "appearance.render.frame.split",
        "prefs.skin_dials.tint",
        "prefs.spiral.zoom",
        "camera_presets[0].yaw",
        "look_slots.stored[1].view.shadow.lattice_text.width",
        "analyzer_regions.geometry.depths",
    ] {
        assert!(pairs.iter().any(|(path, _)| path == deep), "the sweep never reaches {deep}");
    }

    // Each group's bare default, by its path within the group.
    let nested: Vec<_> = nested_defaults()
        .into_iter()
        .map(|(owner, bare, _)| {
            let keys: std::collections::HashMap<_, _> = nested_pairs(&bare)
                .into_iter()
                .map(|(path, range)| (path, bare[range].to_string()))
                .collect();
            (owner, keys)
        })
        .collect();
    let (mut needed, mut stale_values) = (std::collections::HashSet::new(), Vec::new());
    for (path, range) in &pairs {
        // A blob without the editor's `version` reads as 0, which the floor
        // refuses whole on purpose.
        let refused = path == "version";
        // The pair and the comma that separates it from a neighbour.
        let cut = if saved[range.end..].starts_with(',') {
            range.start..range.end + 1
        } else if saved[..range.start].ends_with(',') {
            range.start - 1..range.end
        } else {
            range.clone()
        };
        let without = format!("{}{}", &saved[..cut.start], &saved[cut.end..]);
        let parsed = ron::from_str::<crate::state::UiPersist>(&without).unwrap_or_else(|e| {
            panic!("dropping {path} sank the whole document instead of costing itself: {e}")
        });
        let again = ron::to_string(&parsed).expect("a persist serializes");
        let (_, back) = nested_pairs(&again)
            .into_iter()
            .find(|(p, _)| p == path)
            .unwrap_or_else(|| panic!("dropping {path} lost the key from the next save"));
        // The dropped key's own value.
        let came_back = &again[back.clone()];
        // A stored look is the appearance's camera, view and spectrum under
        // another name, and a fresh save holds none: its keys come back where
        // the appearance's do.
        let like =
            path.strip_prefix("look_slots.stored[1].").map(|rest| format!("appearance.{rest}"));
        let like = like.as_deref().unwrap_or(path);
        let group = nested.iter().find_map(|(owner, keys)| {
            let inner = like.strip_prefix(owner)?.strip_prefix('.')?;
            Some((owner, &keys[inner]))
        });
        let want = match (group, fresh_pairs.get(like)) {
            (Some((owner, bare)), fresh) => {
                if fresh != Some(bare) {
                    needed.insert(*owner);
                }
                Some(bare)
            }
            (None, fresh) if !refused => fresh,
            _ => None,
        };
        if let Some(want) = want.filter(|want| came_back != *want) {
            stale_values.push(format!("{path}: {came_back} where it should load {want}"));
        }
        // Everything else: put the saved value back where the reparsed one
        // stands and compare.
        let masked =
            format!("{}{}{}", &again[..back.start], &saved[range.clone()], &again[back.end..]);
        assert_eq!(masked, saved, "dropping {path} changed more than {path}");
    }
    assert!(stale_values.is_empty(), "a dropped key loaded the wrong value: {stale_values:#?}");
    let stale: Vec<_> =
        nested.iter().map(|(owner, _)| owner).filter(|owner| !needed.contains(*owner)).collect();
    assert!(
        stale.is_empty(),
        "nested_defaults names groups whose bare default is fresh: {stale:?}"
    );
}

/// Whether a serialized scalar is a float: RON writes every float with a
/// fraction or an exponent, and an integer with neither.
pub(super) fn is_float(text: &str) -> bool {
    text.parse::<f64>().is_ok() && text.contains(['.', 'e', 'E'])
}

/// Every float a filled document holds, made `NaN`, `inf` or `-inf` alone,
/// opens on real numbers through both doors: the editor's `install_document`
/// takes the document, the save that follows holds no non-finite number and
/// is a fixed point of the door, and for a float inside the appearance,
/// `AppearanceDocument::parse`, which take replay and export share, reads it
/// as the editor did.
///
/// What this holds is the shape of every repair at once: a bare `clamp` lets
/// a NaN through, and a NaN that reaches a clamp as its bound panics there, as
/// the project opens. What a repair lands on is each owner's own choice, a
/// fresh value, a bare default or an end of the bar, and is not asked here;
/// that it lands on the bar is `settings_ranges`' `Edge::NaN`.
///
/// Derived from the blob like the missing-key sweep above, so a float added
/// anywhere the editor saves is poisoned without an edit here.
#[test]
fn a_document_naming_any_non_finite_float_opens_on_real_numbers() {
    let saved = filled().save_persist();
    let (_, nested) = nested_pairs(&saved)
        .into_iter()
        .find(|(path, _)| path == "appearance")
        .expect("an appearance");
    let nested = nested.start + "appearance:".len()..nested.end;
    let floats: Vec<_> =
        nested_scalars(&saved).into_iter().filter(|(_, at)| is_float(&saved[at.clone()])).collect();
    // The fixture has to reach every owner the editor saves, or the sweep reads
    // as coverage of what it never poisons.
    for owner in [
        "layout.",
        "analyzer_regions.geometry.",
        "appearance.camera.",
        "appearance.view.shadow.",
        "appearance.spectrum.atmosphere.",
        "appearance.render.",
        "camera_presets[0].",
        "look_slots.stored[1].",
        "prefs.",
    ] {
        assert!(floats.iter().any(|(path, _)| path.starts_with(owner)), "no float under {owner}");
    }
    let non_finite = |blob: &str| -> Vec<String> {
        nested_scalars(blob)
            .into_iter()
            .filter(|(_, at)| ["NaN", "inf", "-inf"].contains(&&blob[at.clone()]))
            .map(|(path, _)| path)
            .collect()
    };
    let mut wrong = Vec::new();
    for (path, at) in &floats {
        for poison in ["NaN", "inf", "-inf"] {
            let splice = |blob: &str, at: std::ops::Range<usize>| {
                format!("{}{poison}{}", &blob[..at.start], &blob[at.end..])
            };
            let mut opened = fresh();
            if !opened.install_document(&splice(&saved, at.clone())) {
                wrong.push(format!("{path} = {poison}: the editor refused the document"));
                continue;
            }
            let again = opened.save_persist();
            let left = non_finite(&again);
            if !left.is_empty() {
                wrong.push(format!("{path} = {poison}: the save still holds {left:?}"));
            }
            let mut reopened = fresh();
            assert!(reopened.install_document(&again), "{path} = {poison}: a save was refused");
            if reopened.save_persist() != again {
                wrong.push(format!("{path} = {poison}: the repair is no fixed point"));
            }
            if nested.contains(&at.start) {
                let inside = at.start - nested.start..at.end - nested.start;
                match AppearanceDocument::parse(&splice(&saved[nested.clone()], inside)) {
                    Ok(parsed) if parsed.serialize() == opened.picture.appearance.serialize() => {}
                    Ok(_) => wrong.push(format!("{path} = {poison}: the two doors disagree")),
                    Err(error) => wrong.push(format!("{path} = {poison}: export refused: {error}")),
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Workspace settings stay alongside one nested appearance document.
#[test]
fn the_persist_blob_carries_exactly_these_top_level_keys() {
    // UiPersist's fields, in declaration order.
    const KEYS: &[&str] = &[
        "version",
        "layout",
        "analyzer_regions",
        "appearance",
        "camera_presets",
        "look_slots",
        "prefs",
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
/// This drops whole sections from a fresh state, whose lists are empty, so the
/// structs inside a list never appear;
/// [`a_persist_blob_missing_any_one_key_at_any_depth_keeps_the_rest`] is the
/// layer in.
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
            restored.install_document(&without),
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
    state.workspace.interaction.prefs.spiral.zoom = 2.75;
    state.workspace.layout.right.lattice = 320.0;
    state.workspace.interaction.prefs.folded_sections.insert("System/Performance".into());
    state.workspace.interaction.prefs.ui_scale = 1.25;
    state.workspace.interaction.prefs.frame_rate = crate::FrameRate::Half;
    assert_ne!(state.save_persist(), editor);
    assert_eq!(state.picture.appearance.serialize(), appearance);
    let mut restored = fresh();
    assert!(restored.install_document(&state.save_persist()));
    assert_eq!(restored.picture.appearance.serialize(), appearance);
    // An editor-only variant this build lacks costs the enclosing save its
    // key, and the separately captured appearance nothing at all.
    let broken_workspace = state.save_persist().replace("Console", "RetiredConsole");
    assert!(restored.install_document(&broken_workspace));
    assert_eq!(restored.picture.appearance.serialize(), appearance);
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

/// Every editor preference comes back from a save dialled, as one struct.
///
/// Destructured without `..`, so a preference added later does not compile
/// here until it is dialled: the blob's key list no longer names each one.
#[test]
fn every_editor_preference_round_trips() {
    let mut state = fresh();
    // Each off its default, so a preference re-derived rather than restored
    // shows up.
    let crate::EditorPrefs {
        folded_sections,
        frame_rate,
        ui_scale,
        frameless,
        skin_dials,
        perf_pos,
        show_perf,
        show_perf_detail,
        spiral,
    } = &mut state.workspace.interaction.prefs;
    folded_sections.insert("System/Performance".into());
    *frame_rate = crate::FrameRate::Third;
    *ui_scale = 1.25;
    *frameless = true;
    // Every dial, not one: the pair check below sees `skin_dials` whole.
    *skin_dials = harmonigraph_scene::skin::SkinDials {
        lightness: 0.10,
        tint_hue: 40.0,
        tint: 0.5,
        accent_hue: 120.0,
        accent_saturation: 0.6,
    };
    *perf_pos = Some(egui::pos2(40.0, 60.0));
    *show_perf = true;
    *show_perf_detail = true;
    *spiral = crate::panes::spiral::SpiralView { zoom: 2.75, look: glam::vec2(0.4, -0.6) };
    let dialled = ron::to_string(&state.workspace.interaction.prefs).unwrap();
    let fresh_pairs = top_level_pairs(&ron::to_string(&crate::EditorPrefs::default()).unwrap());
    for pair in top_level_pairs(&dialled) {
        assert!(!fresh_pairs.contains(&pair), "{} is not dialled", pair.0);
    }
    let dials =
        |d: &harmonigraph_scene::skin::SkinDials| top_level_pairs(&ron::to_string(d).unwrap());
    let fresh_dials = dials(&Default::default());
    for pair in dials(&state.workspace.interaction.prefs.skin_dials) {
        assert!(!fresh_dials.contains(&pair), "skin_dials.{} is not dialled", pair.0);
    }

    let mut restored = fresh();
    assert!(restored.install_document(&state.save_persist()));
    assert_eq!(ron::to_string(&restored.workspace.interaction.prefs).unwrap(), dialled);
}

/// An out-of-range scale — only a hand-edited blob can produce one — is
/// clamped rather than drawn at.
#[test]
fn an_impossible_ui_scale_is_clamped() {
    let ctx = super::probe::themed();
    // A nonsense value falls back to the design size rather than to the end of
    // the range it points at: an infinity is not a request for the largest
    // chrome available, it is a blob that has lost the number.
    for (asked, expected) in
        [(0.01f32, 0.75f32), (99.0, 1.6), (f32::NAN, 1.0), (f32::INFINITY, 1.0)]
    {
        crate::theme::set_ui_scale(&ctx, asked);
        assert_eq!(
            crate::theme::ui_scale(&ctx),
            expected,
            "a scale of {asked} was taken at face value",
        );
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
/// The shape asked here is a ramp wider than its middle leaves: a legal pair
/// of floats that names an illegal picture, which sanitize pulls in. A NaN knob
/// is [`a_document_naming_any_non_finite_float_opens_on_real_numbers`]'s.
#[test]
fn a_blob_with_a_nonsense_heatmap_gradient_loads_at_a_drawable_one() {
    for (key, broken) in [("lightness", "5.0")] {
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
        restored.install_document(&spliced);
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
    // is. See `install_document` for why that price is paid rather than shimmed.
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
    restored.install_document(&stale);
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
    current.install_document(&saved);
    assert_eq!(current.picture.appearance.camera.yaw, 1.23);
    assert_eq!(current.picture.appearance.view.max_sevens, 3);
}
/// The tempered links belong to the configuration, not the look (#1677): an
/// editor save carries none, and loading one leaves the session's links as
/// they were — even a save written while the appearance still held its own
/// `meantone` and `marvel`, which this build ignores.
#[test]
fn loading_a_project_leaves_the_links_with_the_configuration() {
    let mut state = fresh();
    let saved = state.save_persist();
    assert!(!saved.contains("meantone") && !saved.contains("marvel"), "{saved}");
    let unlinked = harmonigraph_core::Tempered { syntonic: false, septimal_kleisma: false };
    assert_ne!(state.picture.runtime.tempered, unlinked, "the fixture must change the links");
    for comma in harmonigraph_core::Comma::ALL {
        let tuning = state.picture.runtime.tuning;
        let edit = harmonigraph_core::configuration::ConfigEdit::temper(comma, false, tuning);
        state.picture.runtime.edit_tuning(&Defaults, edit);
    }
    begin_frame(&mut state.picture, &Defaults, 0.0);
    assert_eq!(state.picture.runtime.tempered, unlinked);

    let anchor = pair_starts(&saved, "render_scale");
    assert_eq!(anchor.len(), 1);
    let old = format!("{}meantone:true,marvel:true,{}", &saved[..anchor[0]], &saved[anchor[0]..]);
    assert!(state.install_document(&old), "an old save holding the links must still load");
    for frame in 1..3 {
        begin_frame(&mut state.picture, &Defaults, f64::from(frame));
        assert_eq!(state.picture.runtime.tempered, unlinked, "the save reached the links");
    }
}

/// The Stars resolution round-trips on both textures and through the offline
/// document, an off-grid one snapped to the quarter the slider shows.
#[test]
fn star_resolution_snaps_to_the_slider_at_both_doors() {
    let mut state = fresh();
    state.picture.appearance.view.atmosphere.stars.star_jitter = 0.37;
    state.picture.appearance.view.atmosphere.stars.star_resolution = 0.6;
    state.picture.appearance.view.atmosphere.material_style =
        harmonigraph_scene::LatticeMaterial::Stars;
    state.picture.appearance.spectrum.atmosphere.stars.star_resolution = 0.25;
    let saved = state.save_persist();
    let mut editor = fresh();
    assert!(editor.install_document(&saved));
    assert_eq!(editor.picture.appearance.view.atmosphere.stars.star_jitter, 0.37);
    assert_eq!(editor.picture.appearance.view.atmosphere.stars.star_resolution, 0.5);
    assert_eq!(
        editor.picture.appearance.view.atmosphere.material_style,
        harmonigraph_scene::LatticeMaterial::Stars
    );
    assert_eq!(editor.picture.appearance.spectrum.atmosphere.stars.star_resolution, 0.25);
    let offline = crate::AppearanceDocument::parse(&state.picture.appearance.serialize()).unwrap();
    assert_eq!(offline.view.atmosphere.stars.star_resolution, 0.5);
    assert_eq!(offline.spectrum.atmosphere.stars.star_resolution, 0.25);
}

/// A settings section folded in the editor stays folded across the window
/// closing and reopening.
///
/// The plugin builds a brand-new egui `Context` for every window it opens, so a
/// fold kept in egui memory springs open again with the window (see
/// `shell::Opening`). The folds live in `UiPersist`
/// instead, and this holds the whole path: a REAL click on a section header in
/// the dock, `save_persist`, then a fresh `Context` and `install_document`. Both
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
        state.workspace.interaction.prefs.folded_sections.contains("Analyzer/Spectrogram"),
        "the click did not reach the persisted field: {:?}",
        state.workspace.interaction.prefs.folded_sections,
    );
    assert!(!drawn(&out, leaf, "Softness"), "the click did not fold the section");
    let saved = state.save_persist();

    // The window closes and reopens: a FRESH `Context`, and the state the
    // host hands back through `install_document`.
    let mut reopened = fresh();
    assert!(reopened.install_document(&saved), "the blob this build saved must load");
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
        state.workspace.interaction.prefs.folded_sections.insert(section.to_owned());
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
    pair_ranges(blob).into_iter().map(|(key, range)| (key, blob[range].to_string())).collect()
}

/// [`top_level_pairs`], with each whole pair as its byte range in `blob`.
fn pair_ranges(blob: &str) -> Vec<(String, std::ops::Range<usize>)> {
    item_ranges(blob)
        .into_iter()
        .map(|range| {
            let text = &blob[range.clone()];
            (text[..text.find(':').expect("a pair has a colon")].to_string(), range)
        })
        .collect()
}

/// The top-level items of a serialized struct, tuple or list, as byte ranges
/// in `blob`. Depth-aware, and a quoted string's characters count for nothing.
fn item_ranges(blob: &str) -> Vec<std::ops::Range<usize>> {
    let lead = blob.len() - blob.trim_start().len();
    let trimmed = blob.trim();
    // Exactly ONE enclosing paren off each end. Trimming every one of them takes
    // the last field's own closer along with the container's wherever that field
    // is itself a struct — `(zoom:2.75,look:(0.4,-0.6))` comes back with `look`
    // one paren short, which reads as a pair and rebuilds into a blob that will
    // not parse.
    let (inner, base) =
        match trimmed.strip_prefix(['(', '[']).and_then(|s| s.strip_suffix([')', ']'])) {
            Some(inner) => (inner, lead + 1),
            None => (trimmed, lead),
        };
    if inner.is_empty() {
        return Vec::new();
    }
    let (mut out, mut depth, mut start) = (Vec::new(), 0i32, 0usize);
    let (mut quoted, mut escaped) = (false, false);
    for (i, c) in inner.char_indices() {
        if quoted {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => quoted = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push(base + start..base + i);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(base + start..base + inner.len());
    out
}

/// Every `key:value` pair of every struct a serialized blob holds, at any
/// depth, as its path and its byte range in `blob`: through nested structs,
/// tuples and lists, whose items are numbered (`stored[1]`), and through a
/// `Some` or an enum variant's payload at the path of the value it is.
pub(super) fn nested_pairs(blob: &str) -> Vec<(String, std::ops::Range<usize>)> {
    let mut out = Vec::new();
    walk_blob(blob, 0..blob.len(), "", &mut |found| {
        if let Nested::Pair(path, range) = found {
            out.push((path.to_owned(), range));
        }
    });
    out
}

/// Every scalar a serialized blob holds at any depth — a number, string,
/// unit variant or `None` — as its path and its byte range in `blob`, on the
/// paths [`nested_pairs`] names, so a list's items are `star_speed[0]` and a
/// `Some`'s payload is at the option's own path.
pub(super) fn nested_scalars(blob: &str) -> Vec<(String, std::ops::Range<usize>)> {
    let mut out = Vec::new();
    walk_blob(blob, 0..blob.len(), "", &mut |found| {
        if let Nested::Scalar(path, range) = found {
            out.push((path.to_owned(), range));
        }
    });
    out
}

/// What [`walk_blob`] meets: a whole `key:value` pair, or a value with nothing
/// inside it to walk.
enum Nested<'a> {
    Pair(&'a str, std::ops::Range<usize>),
    Scalar(&'a str, std::ops::Range<usize>),
}

fn walk_blob(blob: &str, value: std::ops::Range<usize>, path: &str, visit: &mut dyn FnMut(Nested)) {
    fn named_struct(text: &str) -> bool {
        let Some(inner) = text.strip_prefix('(') else { return false };
        let key = inner.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(0);
        key > 0 && inner[key..].starts_with(':')
    }
    let text = &blob[value.clone()];
    let name = text.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(0);
    if name > 0 && text[name..].starts_with('(') {
        // A struct variant's payload is its struct; a `Some` or other
        // newtype's single item is the value itself.
        let payload = value.start + name..value.end;
        let items = item_ranges(&blob[payload.clone()]);
        return match items.as_slice() {
            [one] if !named_struct(&blob[payload.clone()]) => {
                walk_blob(blob, payload.start + one.start..payload.start + one.end, path, visit)
            }
            _ => walk_blob(blob, payload, path, visit),
        };
    }
    if named_struct(text) {
        for (name, range) in pair_ranges(text) {
            let range = value.start + range.start..value.start + range.end;
            let path = if path.is_empty() { name.clone() } else { format!("{path}.{name}") };
            visit(Nested::Pair(&path, range.clone()));
            walk_blob(blob, range.start + name.len() + 1..range.end, &path, visit);
        }
    } else if text.starts_with(['(', '[']) {
        for (index, range) in item_ranges(text).into_iter().enumerate() {
            let range = value.start + range.start..value.start + range.end;
            walk_blob(blob, range, &format!("{path}[{index}]"), visit);
        }
    } else {
        visit(Nested::Scalar(path, value));
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
        restored.install_document(&edited);
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
