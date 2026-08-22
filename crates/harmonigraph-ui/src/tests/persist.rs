//! What survives a session: round-trips through [`crate::state::UiPersist`], the version
//! floor under it, and what a blob costs when a key it carries — or one it is
//! missing — is not the shape this build expects.

use crate::*;
use crate::state::UI_PERSIST_VERSION;
use harmonigraph_scene::{Camera, NoteNames};
use super::probe::fresh;

#[test]
fn persist_round_trips_camera_and_view() {
    let mut state = fresh();
    state.camera.yaw = 1.23;
    state.camera.distance = 18.0;
    state.view.extent_sevens = 3;
    // Non-default values throughout, so the fields prove they
    // round-trip rather than matching the defaults by luck.
    state.view.band_width = 0.7;
    state.view.spectral_ring_width = 0.1;
    state.view.ring_gap = 0.02;
    // Melody alone: both marks on is the default, and this test's whole
    // point is that the fields prove they round-trip rather than
    // matching the defaults by luck.
    state.view.mark_melody = true;
    state.view.mark_bass = false;
    // A wheel that is neither the default count nor a center on a C, so the
    // pair proves it round-trips rather than landing back on something the
    // layout would have produced anyway. The center carries a fraction of a
    // semitone the bar cannot set, since the field is a continuous pitch and
    // a blob is entitled to one.
    state.view.octave_count = 7;
    state.view.octave_center = 64.5;
    // A fringe too, with a blend the strip can only reach once there are two
    // extras a side — the three fields are set together because a wheel is
    // what they mean together.
    state.view.octave_extras = 2;
    state.view.octave_extra_size = 0.4;
    state.view.octave_extra_blend = 0.5;
    state.view.grid_thickness = 2.5;
    state.view.grid_inset = 0.0;
    // Which nodes are named, and the fresh view keeps the past -- so either
    // other mode is a value a project has to keep, and the one a fresh view
    // would overwrite if it did not.
    state.view.note_names = NoteNames::All;
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

    let mut restored = fresh();
    restored.load_persist(&saved);
    assert_eq!(restored.camera.yaw, 1.23);
    assert_eq!(restored.camera.distance, 18.0);
    assert_eq!(restored.view.extent_sevens, 3);
    assert_eq!(restored.view.band_width, 0.7);
    assert_eq!(restored.view.spectral_ring_width, 0.1);
    assert_eq!(restored.view.ring_gap, 0.02);
    assert!(restored.view.mark_melody);
    assert!(!restored.view.mark_bass, "bass off round-trips");
    assert_eq!((restored.view.octave_count, restored.view.octave_center), (7, 64.5));
    assert_eq!(restored.view.octave_extras, 2, "the fringe round-trips");
    assert_eq!(restored.view.octave_extra_size, 0.4);
    assert_eq!(restored.view.octave_extra_blend, 0.5);
    assert_eq!(restored.view.grid_thickness, 2.5);
    assert_eq!(restored.view.grid_inset, 0.0, "0 (lines to the center) round-trips");
    assert_eq!(
        restored.view.note_names,
        NoteNames::All,
        "a non-default note-name mode round-trips",
    );
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
    let mut state = fresh();
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

    let mut restored = fresh();
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

/// A hand-edited blob can name a count and a fringe that do not fit the
/// eleven slices the boundary table holds, and neither field is illegal on its
/// own — which is why `sanitize` clamps the PAIR rather than each of them.
/// Clamping only the count would leave the panes showing a fringe the picture
/// does not draw, since the layout re-clamps for itself and says nothing.
#[test]
fn a_blob_naming_more_wheel_than_fits_opens_on_what_fits() {
    let mut state = fresh();
    state.camera.yaw = 1.23;
    state.view.octave_count = 9;
    state.view.octave_extras = 0;
    let saved = state.save_persist();
    // Nine full-size octaves leave room for one extra a side, not five.
    let overrun = saved.replace("octave_extras:0,", "octave_extras:5,");
    assert_ne!(overrun, saved, "`octave_extras` is not in the blob to overrun");

    let mut restored = fresh();
    restored.load_persist(&overrun);
    assert_eq!(
        (restored.view.octave_count, restored.view.octave_extras),
        (9, 1),
        "the count wins and the fringe yields to what is left"
    );
    assert_eq!(restored.camera.yaw, 1.23, "the rest of the blob still restores");
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
        state.camera.yaw = 1.23;
        let saved = state.save_persist();
        let edited = saved
            .replace(
                &format!("floor_db:{:?},", state.spectrum_config.floor_db),
                &format!("floor_db:{floor},"),
            )
            .replace(
                &format!("ceiling_db:{:?},", state.spectrum_config.ceiling_db),
                &format!("ceiling_db:{ceiling},"),
            );
        assert_ne!(edited, saved, "{hint}: the level keys are not in the blob to edit");

        let mut restored = fresh();
        restored.load_persist(&edited);
        let cfg = restored.spectrum_config;
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
        assert_eq!(restored.camera.yaw, 1.23, "{hint}: the rest of the blob still restores");
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
/// It is also the gesture most likely to be aimed here by habit: both controls
/// take a six-digit number that gets captured out of a project rather than
/// dragged, and on a `ValueBar` a double-click opens the box to type one.
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
                    0.5,
                    "Clearance",
                    (fresh.sevens_gutter, fresh.sevens_gutter_soft),
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
        (fresh.sevens_gutter, fresh.sevens_gutter_soft),
        "a double-click landed on ({reach}, {fade}) rather than the fresh pair",
    );
}

/// Every soft edge — the lattice's knockout gutter, the roll's note outline,
/// and the lead a held note carries over the now-line — opens on a pair its own
/// bar can reach: a fade no wider than the reach it is measured back from.
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
    state.camera.yaw = 1.23;
    let saved = state.save_persist();
    let edited = saved
        .replace(
            &format!("sevens_gutter_soft:{:?},", state.view.sevens_gutter_soft),
            "sevens_gutter_soft:0.5,",
        )
        .replace(
            &format!("sevens_gutter:{:?},", state.view.sevens_gutter),
            "sevens_gutter:0.1,",
        )
        .replace(
            &format!("roll_outline_fade:{:?},", state.spectrum_config.roll_outline_fade),
            "roll_outline_fade:9.0,",
        )
        .replace(
            &format!("roll_outline:{:?},", state.spectrum_config.roll_outline),
            "roll_outline:1.0,",
        )
        .replace(
            &format!("roll_lead_fade:{:?},", state.spectrum_config.roll_lead_fade),
            "roll_lead_fade:0.2,",
        )
        .replace(
            &format!("roll_lead:{:?},", state.spectrum_config.roll_lead),
            "roll_lead:0.05,",
        );
    assert_ne!(edited, saved, "the edge keys are not in the blob to edit");

    let mut restored = fresh();
    restored.load_persist(&edited);
    assert_eq!(
        (restored.view.sevens_gutter, restored.view.sevens_gutter_soft),
        (0.1, 0.1),
        "the gutter's fade opened wider than the gutter",
    );
    assert_eq!(
        (restored.spectrum_config.roll_outline, restored.spectrum_config.roll_outline_fade),
        (1.0, 1.0),
        "the outline's fade opened wider than the outline",
    );
    assert_eq!(
        (restored.spectrum_config.roll_lead, restored.spectrum_config.roll_lead_fade),
        (0.05, 0.05),
        "the lead's fade opened wider than the lead",
    );
    assert_eq!(restored.camera.yaw, 1.23, "the rest of the blob still restores");
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
#[test]
fn a_blob_naming_a_nonsense_soft_edge_opens_on_a_drawable_one() {
    let cases: [(&str, &str, &str); 6] = [
        ("sevens_gutter", "NaN", "a NaN gutter"),
        ("sevens_gutter_soft", "NaN", "a NaN gutter fade"),
        ("roll_outline", "inf", "an infinite outline"),
        ("roll_outline_fade", "NaN", "a NaN outline fade"),
        ("roll_lead", "NaN", "a NaN lead"),
        ("roll_lead_fade", "inf", "an infinite lead fade"),
    ];
    for (key, value, hint) in cases {
        let mut state = fresh();
        state.camera.yaw = 1.23;
        let saved = state.save_persist();
        let was = match key {
            "sevens_gutter" => state.view.sevens_gutter,
            "sevens_gutter_soft" => state.view.sevens_gutter_soft,
            "roll_outline" => state.spectrum_config.roll_outline,
            "roll_outline_fade" => state.spectrum_config.roll_outline_fade,
            "roll_lead" => state.spectrum_config.roll_lead,
            _ => state.spectrum_config.roll_lead_fade,
        };
        let edited = saved.replace(&format!("{key}:{was:?},"), &format!("{key}:{value},"));
        assert_ne!(edited, saved, "{hint}: `{key}` is not in the blob to edit");

        let mut restored = fresh();
        restored.load_persist(&edited);
        let view = &restored.view;
        let cfg = &restored.spectrum_config;
        for (name, v) in [
            ("sevens_gutter", view.sevens_gutter),
            ("sevens_gutter_soft", view.sevens_gutter_soft),
            ("roll_outline", cfg.roll_outline),
            ("roll_outline_fade", cfg.roll_outline_fade),
            ("roll_lead", cfg.roll_lead),
            ("roll_lead_fade", cfg.roll_lead_fade),
        ] {
            assert!(v.is_finite(), "{hint}: `{name}` opened at {v}");
        }
        assert!(
            view.sevens_gutter_soft <= view.sevens_gutter
                && cfg.roll_outline_fade <= cfg.roll_outline
                && cfg.roll_lead_fade <= cfg.roll_lead,
            "{hint}: opened on a fade wider than its reach",
        );
        assert_eq!(restored.camera.yaw, 1.23, "{hint}: the rest of the blob still restores");
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
        state.camera.yaw = 1.23;
        let saved = state.save_persist();
        let was = state.spectrum_config.roll_lead_release;
        let edited = saved.replace(
            &format!("roll_lead_release:{was:?},"),
            &format!("roll_lead_release:{value},"),
        );
        assert_ne!(edited, saved, "{hint}: `roll_lead_release` is not in the blob to edit");

        let mut restored = fresh();
        restored.load_persist(&edited);
        assert_eq!(
            restored.spectrum_config.roll_lead_release, want,
            "{hint}: opened at {}",
            restored.spectrum_config.roll_lead_release,
        );
        assert_eq!(restored.camera.yaw, 1.23, "{hint}: the rest of the blob still restores");
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
            state.camera.yaw = 1.23;
            let saved = state.save_persist();
            let edited =
                saved.replace(&format!("{key}:{fresh_value:?},"), &format!("{key}:{value},"));
            assert_ne!(edited, saved, "{hint}: `{key}` is not in the blob to edit");

            let mut restored = fresh();
            restored.load_persist(&edited);
            let got = if key == "attack" {
                restored.spectrum_config.attack
            } else {
                restored.spectrum_config.release
            };
            assert_eq!(got, want, "{hint}: `{key}` opened at {got}");
            assert_eq!(restored.camera.yaw, 1.23, "{hint}: the rest of the blob still restores");
        }
    }
}

/// The wheel's two-bar TAPER is gone, and a blob carrying the pair of keys
/// nothing reads now keeps everything else it says. An unknown field being
/// ignored rather than refused is the whole of why that works, and it is a
/// property of how the blob is read rather than anything this crate spells
/// out, so it is worth a blob to say so.
///
/// The fringe the taper predates is dropped from the same blob, which is the
/// other half of the read: a key that is ABSENT costs its own field and comes
/// back at the fresh value, where a key that is UNKNOWN costs nothing at all.
#[test]
fn a_blob_written_against_the_taper_keeps_what_it_still_says() {
    let mut state = fresh();
    state.camera.yaw = 1.23;
    state.view.octave_count = 7;
    state.view.octave_extras = 3;
    state.view.octave_extra_size = 0.4;
    state.view.octave_extra_blend = 0.5;
    let mut saved = state.save_persist();
    // Exactly a pre-fringe blob: none of the three keys the fringe added, and
    // the two the taper wrote where they now sit. One key at a time, each
    // checked to have hit, so a rename cannot leave a default untested.
    for key in [
        format!("octave_extras:{},", state.view.octave_extras),
        format!("octave_extra_size:{:?},", state.view.octave_extra_size),
        format!("octave_extra_blend:{:?},", state.view.octave_extra_blend),
    ] {
        let stripped = saved.replace(&key, "");
        assert_ne!(stripped, saved, "{key:?} is not in the blob to remove");
        saved = stripped;
    }
    let count = format!("octave_count:{},", state.view.octave_count);
    let tapered =
        saved.replace(&count, &format!("{count}octave_taper_amount:0.6,octave_taper_shape:0.25,"));
    assert_ne!(tapered, saved, "the taper's keys did not go into the blob");

    let defaults = harmonigraph_scene::ViewConfig::default();
    let mut restored = fresh();
    restored.load_persist(&tapered);
    assert_eq!(restored.view.octave_count, 7, "the count the blob names survives the taper keys");
    // The fresh fringe as this count can hold it, not the fresh fringe raw:
    // `sanitize` clamps the PAIR, and seven octaves leave room for two extras
    // a side. Spelling the clamp out keeps this measuring the fallback rather
    // than failing the day someone retunes the fresh fringe past what fits.
    let (_, fits) = harmonigraph_scene::clamp_wheel(7, defaults.octave_extras);
    assert_eq!(
        restored.view.octave_extras, fits,
        "and the fringe it is missing comes back at the fresh value, as the count can hold it",
    );
    assert_eq!(restored.camera.yaw, 1.23, "the rest of the blob still restores");
}

/// The octave glyphs' own shimmer is gone, and a project saved against it
/// carries a `pulse_octaves` key nothing reads now. It has to open with the
/// glyphs steady and every other setting intact — including the marks'
/// pattern, which is the one that survived and which sits next to it in the
/// blob, so a reader that choked on the retired key would take the live one
/// down with it.
///
/// Worth a blob rather than a comment for the same reason the taper's is: that
/// an unknown field is ignored rather than refused is a property of how the
/// blob is read, not something this crate spells out, and a saved project
/// failing to parse loses the user's layout and camera along with it.
#[test]
fn a_blob_written_against_the_octave_shimmer_opens_with_the_glyphs_steady() {
    let mut state = fresh();
    state.camera.yaw = 1.23;
    state.view.pulse_marks = harmonigraph_scene::Pulse::Hex;
    let saved = state.save_persist();
    // Where the retired key sat: beside the marks' own pattern, written
    // bare as RON writes a unit variant.
    let marks = "pulse_marks:Hex,";
    let with_octaves = saved.replace(marks, &format!("pulse_octaves:Bands,{marks}"));
    assert_ne!(with_octaves, saved, "`{marks}` is not in the blob to splice against");

    let mut restored = fresh();
    restored.load_persist(&with_octaves);
    assert_eq!(
        restored.view.pulse_marks,
        harmonigraph_scene::Pulse::Hex,
        "the pattern that survived the retirement came back changed",
    );
    assert_eq!(restored.camera.yaw, 1.23, "the rest of the blob still restores");
    assert!(
        !restored.save_persist().contains("pulse_octaves"),
        "the retired key was written back out; it should drop on the next save",
    );
}

/// The render frame round-trips its side and the split beside it, through
/// BOTH doors into the blob — the editor's and the offline renderer's.
#[test]
fn a_render_frame_round_trips_through_both_doors() {
    let mut state = fresh();
    state.camera.yaw = 1.23;
    state.take.render_config.frame.lattice = LatticeSide::Bottom;
    state.take.render_config.frame.split = 0.42;
    let saved = state.save_persist();
    assert!(saved.contains("lattice:Bottom"), "the side is what gets written");

    let mut restored = fresh();
    restored.load_persist(&saved);
    assert_eq!(restored.take.render_config.frame.lattice, LatticeSide::Bottom);
    assert_eq!(restored.take.render_config.frame.split, 0.42);
    assert_eq!(restored.camera.yaw, 1.23, "rest of the blob still restores");

    let render = crate::render_config_from_persist(&saved).expect("still parses");
    assert_eq!(render.frame.lattice, LatticeSide::Bottom, "through render_config_from_persist");
    assert_eq!(render.frame.split, 0.42);
}

#[test]
fn corrupt_persist_is_ignored() {
    let mut state = fresh();
    let default_distance = state.camera.distance;
    assert!(!state.load_persist("not json at all"), "a corrupt blob is not applied");
    assert_eq!(state.camera.distance, default_distance);
}

/// A refused blob SAYS SO. Both refusals cost the whole document — dock,
/// camera, view, spectrum and render at once — and a project reopening at
/// defaults with nothing written anywhere reads as data loss rather than as a
/// break someone chose.
///
/// The variant case is the one worth a test of its own, because it is the one
/// this build can still meet in the wild: no code reads an older spelling any
/// more, so a blob naming an orientation or sweep mode that has since been
/// dropped fails the parse — and it fails BEFORE the version is read, so the
/// floor cannot catch it however high it is set.
#[test]
fn a_refused_blob_says_why() {
    // A dropped enum variant, spliced where a live one sat. The orientation,
    // the heatmap's look being six numbers rather than an enum — and a number
    // out of range is repaired rather than refused, which is the other test.
    let mut state = fresh();
    state.spectrum_config.orientation = crate::SpectralOrientation::Left;
    let saved = state.save_persist();
    let dropped = saved.replace("orientation:Left", "orientation:Diagonal");
    assert_ne!(dropped, saved, "the splice must land for this to test anything");

    let mut restored = fresh();
    assert!(!restored.load_persist(&dropped), "a dropped variant is not applied");
    assert!(
        restored.console.lines().any(|line| line.contains("did not parse")),
        "the refusal was silent; console holds {:?}",
        restored.console.lines().collect::<Vec<_>>(),
    );

    // And the floor's own refusal, which is the other way a whole document
    // goes and must be just as loud.
    let stale = saved.replacen(
        &format!("version:{UI_PERSIST_VERSION}"),
        &format!("version:{}", UI_PERSIST_VERSION - 1),
        1,
    );
    let mut older = fresh();
    assert!(!older.load_persist(&stale), "a blob below the floor is not applied");
    assert!(
        older.console.lines().any(|line| line.contains("below the floor")),
        "the floor's refusal was silent",
    );
}

#[test]
fn spectrum_config_round_trips_through_persist() {
    let mut state = fresh();
    state.spectrum_config.floor_db = -48.0;
    state.spectrum_config.ceiling_db = -12.0;
    state.spectrum_config.window = SpectrumWindow::Precise;
    state.spectrum_config.low_midi = 40.5;
    state.spectrum_config.show_spectrogram = true;
    // A gradient no preset writes, so what round-trips is the six numbers and
    // not a name that happens to rebuild them.
    state.spectrum_config.spectrogram_gradient = harmonigraph_scene::Gradient {
        hue_start: 137.5,
        hue_span: -85.25,
        lightness: 44.0,
        lightness_ramp: 71.5,
        chroma: 0.375,
        chroma_ramp: -0.25,
    };
    let saved = state.save_persist();

    let mut restored = fresh();
    restored.load_persist(&saved);
    assert_eq!(restored.spectrum_config.floor_db, -48.0);
    assert_eq!(restored.spectrum_config.ceiling_db, -12.0);
    assert_eq!(restored.spectrum_config.window, SpectrumWindow::Precise);
    // A range off the C boundaries survives, which the octave pair could not
    // have expressed at all.
    assert_eq!(restored.spectrum_config.low_midi, 40.5);
    assert!(restored.spectrum_config.show_spectrogram);
    assert_eq!(
        restored.spectrum_config.spectrogram_gradient,
        state.spectrum_config.spectrogram_gradient,
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
        state.spectrum_config.low_midi = 60.0;
        state.spectrum_config.high_midi = 72.0;
        let saved = state
            .save_persist()
            .replace("low_midi:60.0", &format!("low_midi:{low}"))
            .replace("high_midi:72.0", &format!("high_midi:{high}"));
        let mut restored = fresh();
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
    let mut state = fresh();
    state.spectrum_config.show_spectrogram = false;
    let saved = state.save_persist();
    let old = saved.replace("show_spectrogram:false,", "");
    assert_ne!(old, saved, "the field must have been there to strip");

    let mut restored = fresh();
    restored.load_persist(&old);
    assert!(
        restored.spectrum_config.show_spectrogram,
        "a missing field must fall back to the struct's own default, not bool::default()"
    );
    // An explicit `false` is a choice, not an absence, and still round-trips.
    let mut restored = fresh();
    restored.load_persist(&saved);
    assert!(!restored.spectrum_config.show_spectrogram);
}

/// Splice `spliced` in ahead of the `anchor` key of a real blob and check the
/// blob still comes through BOTH doors. A key that has since been removed must
/// not take the whole blob down with it: a blob that fails to parse loses the
/// entire UI state, not just the stale key, so every settings removal rides on
/// serde ignoring what it has no field for, and this is where that is held.
///
/// Both doors, because only one of them is the editor's. An offline take reads
/// the same blob through `render_config_from_persist`, and
/// `both_doors_into_a_blob_agree_about_the_version_floor` is what keeps the two
/// in step. They parse the same `UiPersist` today, so a stale key is skipped
/// identically and the second assertion is redundant — but narrowing the
/// offline door to a partial struct is the obvious optimization (it wants only
/// `render`), and that is exactly what would let a key it has no field for sink
/// a take while the editor went on loading.
fn a_spliced_blob_survives_both_doors(anchor: &str, spliced: &str) {
    let mut state = fresh();
    // Non-defaults on both sides of the splice, so "the blob survived" is
    // distinguishable from "it sank and everything reverted": the view is what
    // the editor door restores, the lead-in what the offline door reads.
    state.view.extent_sevens = 3;
    state.take.render_config.lead_in = 2.5;
    let saved = state.save_persist();
    let stale = saved.replace(anchor, &format!("{spliced}{anchor}"));
    assert_ne!(stale, saved, "the anchor field must have been there to splice onto");

    let mut restored = fresh();
    restored.load_persist(&stale);
    assert_eq!(restored.view.extent_sevens, 3, "an unknown key must not sink the blob");

    let offline = crate::render_config_from_persist(&stale)
        .expect("an unknown key must not sink the offline door either");
    assert_eq!(offline.lead_in, 2.5, "the offline door must read past an unknown key");
}

/// The numeric case. `spectrogram_fine_levels` existed only while the heatmap's
/// stored precision was being judged by eye; the other five are the heatmap's
/// opacity, contrast and private dB window, which every project saved before
/// they were dropped still carries.
///
/// `spectrogram_color` rides with them, and it is the case that makes this test
/// worth more than a sweep of numbers: it is an ENUM TOKEN, so a reader strict
/// about keys it has no field for would need a type to parse `Aurora` into, and
/// there is none any more — the heatmap's look is a gradient now. Every project
/// saved before that carries the key.
#[test]
fn a_persist_blob_carrying_a_since_removed_field_still_loads() {
    // Put the departed fields back, exactly as those builds wrote them.
    a_spliced_blob_survives_both_doors(
        "spectrogram_gradient:",
        "spectrogram_color:Aurora,spectrogram_fine_levels:true,\
         spectrogram_opacity:0.85,spectrogram_own_range:true,\
         spectrogram_floor_db:-60.0,spectrogram_ceiling_db:-20.0,\
         spectrogram_gamma:1.6,",
    );
}

/// The same rule, for the key every project above the version floor carries:
/// the core's paint was a choice of styles, and `node_style` names whichever of
/// the seventeen the enum answered to that project was drawn with. It gets its
/// own case because what it carries is an ENUM TOKEN and not a number — the
/// shape that would need a type to parse into if the reader were strict about
/// what it has no field for.
///
/// Both tokens run. `Vortex` is a style that survived to the end; `Pinwheel` is
/// one only an alias kept loading. They are equally unknown now, which is a
/// claim the pair has to make by BEING run — asserting it in a comment over a
/// body that splices one of them is how the two drift apart.
#[test]
fn a_persist_blob_naming_a_retired_node_style_still_loads() {
    // Where the key sat, written as those builds wrote it.
    for token in ["Vortex", "Pinwheel"] {
        a_spliced_blob_survives_both_doors("pitch_gradient:", &format!("node_style:{token},"));
    }
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
        format!("window:{:?},", defaults.window),
    ] {
        let mut state = fresh();
        // A non-default elsewhere in the blob, so "the blob survived" is
        // distinguishable from "it sank and everything reverted".
        state.view.extent_sevens = 3;
        let saved = state.save_persist();
        let without = saved.replacen(key.as_str(), "", 1);
        assert_ne!(without, saved, "{key:?} must be in the blob to drop");

        let mut restored = fresh();
        restored.load_persist(&without);
        assert_eq!(
            restored.view.extent_sevens, 3,
            "dropping {key:?} must cost that key alone, not the whole blob",
        );
        assert_eq!(
            restored.spectrum_config, defaults,
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
        "display_page",
        "camera",
        "view",
        "camera_presets",
        "spectrum",
        "spiral",
        "render",
        "fps_cap",
        "ui_scale",
        "perf_pos",
    ];

    let saved = fresh().save_persist();
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
/// No blob this build WROTE is in that shape: `render` entered [`UiPersist`]
/// before [`UI_PERSIST_VERSION`] was raised past the versions that predate it,
/// so a saved project missing the section is below the floor and refused whole
/// before its `#[serde(default)]` is ever consulted.
///
/// A HAND-AUTHORED blob is the reachable case, and it is a supported one, not
/// a curiosity: `harmonigraph-offline --ui-state FILE` substitutes a file for
/// the take's own blob without validating it, and the standalone reads its
/// `app.ron` the same way. A file dialled by hand — or by
/// `read-plugin-state.py`, which the flag's own help points at — is exactly
/// the blob that can be missing a section, and dropping one there must not
/// sink the other nine.
///
/// Both doors, because they answer separately: `load_persist` for the rest of
/// the settings, and `render_config_from_persist` for the frame the offline
/// renderer composes at. The renderer does have a fallback behind that door
/// (`main`'s `unwrap_or_default`), so what this holds is the two doors
/// AGREEING about one blob — the property
/// `both_doors_into_a_blob_agree_about_the_version_floor` holds at the version.
#[test]
fn a_persist_blob_missing_the_render_section_keeps_the_rest_of_the_blob() {
    let mut state = fresh();
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

    let mut restored = fresh();
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

/// The same property for EVERY section that carries the attribute, not just
/// `render`: dropping one costs that section alone.
///
/// Swept rather than pinned one at a time, because nothing at a declaration
/// says whether `#[serde(default)]` is there — `camera` and `view` went
/// without it for a while precisely because the omission is invisible, and a
/// hand-authored `--ui-state` file setting one thing omits most of the rest.
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
    state.camera.yaw = 1.23;
    state.view.extent_sevens = 3;
    let saved = state.save_persist();

    for (key, _) in top_level_pairs(&saved) {
        if key == "version" || key == "dock" || key == "camera" {
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
        assert_eq!(restored.camera.yaw, 1.23, "dropping {key:?} cost the camera too");
    }
}

/// The Spiral pane's framing round-trips, and a hand-edited one comes back
/// drawable.
///
/// Persisted for the reason the camera beside it is — a framing is dialled in by
/// hand and a take renders from the blob — so what has to hold is that the pair
/// SURVIVES rather than being re-derived from the fit, and that a nonsense one
/// cannot reach the pane: both fields multiply the geometry it paints, and NaN
/// geometry is a panic inside egui's tessellator.
#[test]
fn persist_round_trips_the_spiral_framing() {
    let mut state = fresh();
    // A zoom off both ends of the range and a look off both axes, so a field
    // dropped or transposed on the way through shows up.
    state.spiral_view =
        crate::panes::spiral::SpiralView { zoom: 2.75, look: glam::vec2(0.4, -0.6) };

    let mut restored = fresh();
    assert!(restored.load_persist(&state.save_persist()));
    assert_eq!(restored.spiral_view.zoom, 2.75);
    assert_eq!(restored.spiral_view.look, glam::vec2(0.4, -0.6));

    // And the repair on the way in, which is `load_persist`'s call rather than
    // the pane's: a blob nothing but a text editor could have written.
    let saved = state.save_persist();
    let edited = replace_pair(&saved, "zoom", "2.75", "NaN");
    assert_ne!(edited, saved, "`zoom` is not in the blob to edit");
    let mut restored = fresh();
    assert!(restored.load_persist(&edited));
    assert!(
        restored.spiral_view.zoom.is_finite(),
        "a NaN zoom opened at {}",
        restored.spiral_view.zoom,
    );
}

/// Dropping any one key from INSIDE the `spiral` section costs that key alone:
/// the rest of the blob loads, the section's other field keeps the value the blob
/// names, and the dropped one comes back at the fresh-install value.
///
/// One layer in from [`a_persist_blob_missing_any_one_section_keeps_the_rest`],
/// and that is the whole reason it exists: sweeping whole sections exercises
/// `UiPersist::spiral`'s field-level attribute, where this is the only thing that
/// asks after `SpiralView`'s container-level one — the attribute nothing at a
/// declaration says is there.
///
/// The input is one a hand-authored `--ui-state` file arrives in every time:
/// `spiral: (zoom: 3.0)` and no `look`, because a person writing a framing out by
/// hand writes the field they came to change. Without the attribute that file does
/// not cost `look` — it sinks the whole document, dock and camera with it, and
/// nothing but this would fail.
#[test]
fn a_persist_blob_missing_any_one_spiral_key_keeps_the_rest() {
    let mut state = fresh();
    // A witness outside the section, so "the blob survived" is distinguishable
    // from "it sank and every section reverted together".
    state.camera.yaw = 1.23;
    // Both fields off their fresh values, and the look off both axes: a field
    // that came back from the wrong place is only visible against a value the
    // default is not.
    state.spiral_view =
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
        assert_eq!(restored.camera.yaw, 1.23, "dropping the spiral's {key:?} cost the camera too");
        let want = match key.as_str() {
            "zoom" => (opened.zoom, state.spiral_view.look),
            "look" => (state.spiral_view.zoom, opened.look),
            other => panic!("the framing grew a {other:?} field this sweep does not name"),
        };
        assert_eq!(
            (restored.spiral_view.zoom, restored.spiral_view.look),
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
    state.fps_cap = Some(45.0);

    let mut restored = fresh();
    restored.load_persist(&state.save_persist());
    assert_eq!(restored.fps_cap, Some(45.0));
}

#[test]
fn pre_cap_persist_blobs_load_as_uncapped() {
    // The cap was added after these blobs were written; dropping the field
    // must not fail the parse, which would silently discard the WHOLE
    // persist (layout, camera, every view setting) rather than one setting.
    let mut state = fresh();
    state.fps_cap = Some(30.0);
    state.view.extent_sevens = 3;
    let saved = state.save_persist();
    let stripped = saved.replace(",fps_cap:Some(30.0)", "");
    assert_ne!(stripped, saved, "the field removal must have hit");

    let mut restored = fresh();
    restored.load_persist(&stripped);
    assert_eq!(restored.fps_cap, None, "a missing cap reads as uncapped");
    assert_eq!(restored.view.extent_sevens, 3, "the rest of the blob must survive");
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
/// identifier is the shape that has actually cost a blob here. What separates
/// the two is that retiring a FIELD is safe where retiring a VARIANT is not —
/// a blob naming a variant the enum has dropped fails to parse and takes the
/// whole persist with it, which is a break this build accepts rather than
/// carries an alias for — and a test that only ever splices a number cannot
/// tell them apart.
#[test]
fn a_retired_setting_does_not_discard_the_blob_it_was_saved_in() {
    for retired in ["roll_gap:2.5,", "roll_color:Pitch,"] {
        let mut state = fresh();
        state.spectrum_config.roll_thickness = 1.75;
        state.spectrum_config.low_midi = 40.5;
        let saved = state.save_persist();
        // A blob from before the retirement: the key spliced back where it sat.
        let old = saved.replacen("roll_thickness:", &format!("{retired}roll_thickness:"), 1);
        assert_ne!(old, saved, "the {retired} splice must land for this to test anything");

        let mut restored = fresh();
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
    let mut state = fresh();
    state.ui_scale = 0.75;
    let saved = state.save_persist();
    let mut back = fresh();
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
    let mut older = fresh();
    older.load_persist(&stripped);
    assert_eq!(older.ui_scale, 1.0, "a blob with no scale did not load at the design size");
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
        let mut state = fresh();
        // Off the defaults, so the splice has something to name and a range
        // that survives proves it survived rather than matching by luck.
        state.spectrum_config.low_midi = 40.5;
        state.spectrum_config.high_midi = 90.25;
        let saved = state.save_persist();
        let value = if end == "low_midi" { 40.5f32 } else { 90.25f32 };
        let broken = saved.replacen(&format!("{end}:{value:?}"), &format!("{end}:NaN"), 1);
        assert_ne!(broken, saved, "the {end} splice must land for this to test anything");

        let mut restored = fresh();
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
        state.spectrum_config.spectrogram_gradient = harmonigraph_scene::Gradient {
            hue_start: 137.5,
            hue_span: -85.25,
            lightness: 44.0,
            lightness_ramp: 71.5,
            chroma: 0.375,
            chroma_ramp: -0.25,
        };
        let saved = state.save_persist();
        let was = if key == "lightness" { "44.0" } else { "0.375" };
        let spliced = saved.replacen(&format!("{key}:{was}"), &format!("{key}:{broken}"), 1);
        assert_ne!(spliced, saved, "the {key} splice must land for this to test anything");

        let mut restored = fresh();
        restored.load_persist(&spliced);
        let g = restored.spectrum_config.spectrogram_gradient;
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
    // the plugin's CLAP/VST3 ids gate everything below version 2, but nothing
    // gates a blob one bump behind, which a project saved by the previous build
    // is. See `load_persist` for why that price is paid rather than shimmed.
    let mut state = fresh();
    state.camera.yaw = 1.23;
    state.view.extent_sevens = 3;
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
    assert_eq!(restored.camera.yaw, defaults.camera.yaw);
    assert_eq!(restored.view.extent_sevens, defaults.view.extent_sevens);

    // And the same blob at the floor still loads, so the test above is
    // measuring the version rather than a blob that was broken anyway.
    let mut current = fresh();
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
/// identity gate either. Reachable today rather than in principle: the floor
/// stands at 3, so a take recorded before it was raised holds a version-2 blob
/// and re-renders at the default look and frame. That is accepted rather than
/// shimmed, which is what makes the agreement this test pins the thing that
/// matters — both doors have to refuse the same blobs, and say so.
#[test]
fn both_doors_into_a_blob_agree_about_the_version_floor() {
    let mut state = fresh();
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
    let mut restored = fresh();
    restored.load_persist(&stale);
    let defaults = fresh();
    assert_eq!(restored.take.render_config.playhead, defaults.take.render_config.playhead);

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
    let mut state = fresh();
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



/// Dropping any one key from a serialized view costs THAT KEY alone, and the
/// value it comes back with is the fresh-install one.
///
/// [`ViewConfig`](harmonigraph_scene::ViewConfig) carries a container-level
/// `#[serde(default)]`, so `impl Default` is the single source of every
/// field's fallback. That is the whole arrangement, and this is what holds it:
/// nothing at a declaration says whether a field can survive being absent, so
/// the property is probed from the outside instead — rebuild the blob without
/// one key at a time, which is exactly the shape a hand-edited RON or a
/// `--ui-state` file can arrive in, and reload.
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

/// The Display page picked in the editor survives the window closing and
/// reopening.
///
/// The plugin builds a brand-new egui `Context` for every window it opens, so a
/// choice kept in egui memory springs back to the default with the window — the
/// same class of trap as the stale `TextureHandle`
/// (`SharedState::release_context_resources`). The page lives in `UiPersist`
/// instead, and this holds the whole path: a REAL click on the picker in the
/// dock, `save_persist`, then a fresh `Context` and `load_persist`. Both halves
/// are load-bearing — writing the field by hand would pass with the click never
/// wired to it, and asserting inside one `Context` would pass with the state
/// memory-backed, which is the live bug this exists to catch.
#[test]
fn the_display_page_in_the_picker_survives_an_editor_reopen() {
    use super::harness::{press, DockHarness};

    // A text only one page's body draws — the Lattice page's projection row,
    // the Colors page's Bloom bar — scoped to the settings leaf.
    let drawn = |out: &egui::FullOutput, leaf: egui::Rect, needle: &str| {
        out.shapes.iter().any(|cs| match &cs.shape {
            egui::Shape::Text(t) => t.galley.text() == needle && leaf.contains(t.pos),
            _ => false,
        })
    };

    let mut state = fresh();
    let path = state.workspace.dock.find_tab(&panes::Tab::Display).expect("Display is docked");
    state.workspace.dock.set_active_tab(path).expect("selecting the tab");
    let mut window = DockHarness::new();
    window.settle(&mut state);
    let leaf = state.workspace.dock[path.surface][path.node].rect().expect("the leaf is laid out");
    let out = window.frame(&mut state, vec![]);
    assert!(!drawn(&out, leaf, "Projection"), "the tab must open on the Colors page");

    // The Lattice button on the picker, found where it was painted and clicked
    // for real.
    let button = out
        .shapes
        .iter()
        .find_map(|cs| match &cs.shape {
            egui::Shape::Text(t) if t.galley.text() == "Lattice" && leaf.contains(t.pos) => {
                Some(egui::Rect::from_min_size(t.pos, t.galley.size()).center())
            }
            _ => None,
        })
        .expect("the Display pane drew no Lattice button");
    window.frame(&mut state, vec![egui::Event::PointerMoved(button)]);
    window.frame(&mut state, vec![egui::Event::PointerMoved(button), press(button, true)]);
    window.frame(&mut state, vec![press(button, false)]);
    let out = window.frame(&mut state, vec![]);
    assert_eq!(
        state.display_page,
        panes::display::DisplayPage::Lattice,
        "the click did not reach the persisted field",
    );
    assert!(drawn(&out, leaf, "Projection"), "the click did not switch to the Lattice page");
    let saved = state.save_persist();

    // The window closes and reopens: a FRESH `Context`, and the state the
    // host hands back through `load_persist`.
    let mut reopened = fresh();
    assert!(reopened.load_persist(&saved), "the blob this build saved must load");
    let mut fresh_window = DockHarness::new();
    fresh_window.settle(&mut reopened);
    let out = fresh_window.frame(&mut reopened, vec![]);
    let path =
        reopened.workspace.dock.find_tab(&panes::Tab::Display).expect("Display survives the blob");
    let leaf =
        reopened.workspace.dock[path.surface][path.node].rect().expect("the leaf is laid out");
    assert!(
        drawn(&out, leaf, "Projection"),
        "the page reverted across the reopen — is its state in egui memory?",
    );
    // And one page is ONE page: the Colors body it was switched away from is
    // gone rather than still stacked above.
    assert!(
        !drawn(&out, leaf, "Bloom"),
        "the Colors page is still drawn under the Lattice page",
    );
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
/// `screen_scale` already guards `distance`/`fov_y` locally for the
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
    let cases: [CameraCase; 11] = [
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
        ("fov_y", "NaN", "a NaN field of view", Some((0.2, 2.0))),
        ("fov_y", "100.0", "a field of view `ortho`'s `tan` cannot take", Some((0.2, 2.0))),
        (
            "cabinet_angle",
            "NaN",
            "a NaN cabinet angle",
            Some((0.0, std::f32::consts::FRAC_PI_2)),
        ),
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
        state.view.extent_sevens = 3;
        let saved = state.save_persist();
        let was = match key {
            "yaw" => state.camera.yaw,
            "pitch" => state.camera.pitch,
            "distance" => state.camera.distance,
            "fov_y" => state.camera.fov_y,
            "cabinet_angle" => state.camera.cabinet_angle,
            _ => state.camera.cabinet_scale,
        };
        let edited = replace_pair(&saved, key, &format!("{was:?}"), value);
        assert_ne!(edited, saved, "{hint}: `{key}` is not in the blob to edit");

        let mut restored = fresh();
        restored.load_persist(&edited);
        let got = match key {
            "yaw" => restored.camera.yaw,
            "pitch" => restored.camera.pitch,
            "distance" => restored.camera.distance,
            "fov_y" => restored.camera.fov_y,
            "cabinet_angle" => restored.camera.cabinet_angle,
            _ => restored.camera.cabinet_scale,
        };
        assert!(got.is_finite(), "{hint}: `{key}` opened at {got}");
        if let Some((lo, hi)) = range {
            assert!(got >= lo && got <= hi, "{hint}: `{key}` opened at {got}, outside {lo}..={hi}");
        }
        assert_eq!(restored.view.extent_sevens, 3, "{hint}: the rest of the blob still restores");
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
    for (hint, yaw, pitch) in [
        ("a NaN yaw", "NaN", "0.0"),
        ("a pitch past `orbit`'s limit", "0.0", "10.0"),
    ] {
        let mut state = fresh();
        state.view.extent_sevens = 3;
        state.camera_presets.push(crate::CameraPreset {
            name: "reading".to_string(),
            yaw: 0.25,
            pitch: 0.5,
        });
        let saved = state.save_persist();
        let edited = saved.replacen(
            "yaw:0.25,pitch:0.5",
            &format!("yaw:{yaw},pitch:{pitch}"),
            1,
        );
        assert_ne!(edited, saved, "{hint}: the preset is not in the blob to edit");

        let mut restored = fresh();
        restored.load_persist(&edited);
        assert_eq!(restored.view.extent_sevens, 3, "{hint}: the rest of the blob still restores");
        let preset = &restored.camera_presets[0];

        // Applied exactly as the preset button applies it.
        let mut camera = restored.camera;
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
    state.view.extent_sevens = 3;
    let saved = state.save_persist();
    let edited = saved.replace("target:(0.0,0.0,0.0),", "target:(NaN,0.0,0.0),");
    assert_ne!(edited, saved, "`target` is not in the blob to edit");

    let mut restored = fresh();
    restored.load_persist(&edited);
    assert!(
        restored.camera.target.is_finite(),
        "a NaN component opened at {:?}",
        restored.camera.target,
    );
    assert_eq!(restored.view.extent_sevens, 3, "the rest of the blob still restores");
}

/// The Video pane's two numeric dials, on the same footing: `lead_in` feeds
/// the offline renderer's frame count and `split` feeds `Layout::split`,
/// whose own clamp cannot repair a NaN (it loses every comparison a clamp
/// makes), only hold a finite value inside a literal range.
#[test]
fn a_blob_naming_a_nonsense_render_config_opens_on_what_it_can_reach() {
    let cases: [(&str, &str, &str, (f32, f32)); 4] = [
        ("lead_in", "NaN", "a NaN lead-in", (0.0, 5.0)),
        ("lead_in", "999.0", "a lead-in past its bar", (0.0, 5.0)),
        ("split", "NaN", "a NaN split", (0.05, 0.95)),
        ("split", "inf", "an infinite split", (0.05, 0.95)),
    ];
    for (key, value, hint, (lo, hi)) in cases {
        let mut state = fresh();
        state.view.extent_sevens = 3;
        let saved = state.save_persist();
        let was = match key {
            "lead_in" => state.take.render_config.lead_in,
            _ => state.take.render_config.frame.split,
        };
        let edited = replace_pair(&saved, key, &format!("{was:?}"), value);
        assert_ne!(edited, saved, "{hint}: `{key}` is not in the blob to edit");

        let mut restored = fresh();
        restored.load_persist(&edited);
        let got = match key {
            "lead_in" => restored.take.render_config.lead_in,
            _ => restored.take.render_config.frame.split,
        };
        assert!(got.is_finite(), "{hint}: `{key}` opened at {got}");
        assert!(got >= lo && got <= hi, "{hint}: `{key}` opened at {got}, outside {lo}..={hi}");
        assert_eq!(restored.view.extent_sevens, 3, "{hint}: the rest of the blob still restores");
    }
}

/// `render_config_from_persist` is the OTHER door into a `ui_state` blob —
/// `harmonigraph-offline`'s own, for the frame it composes at — and it has
/// to sanitize independently of `load_persist`: a `--ui-state` file is
/// handed straight to it without ever passing through a `SharedState`, so
/// nothing upstream has repaired it.
#[test]
fn a_blob_naming_a_nonsense_split_opens_on_a_drawable_one_through_render_config_from_persist() {
    let state = fresh();
    let saved = state.save_persist();
    let edited = replace_pair(&saved, "split", "0.2", "NaN");
    assert_ne!(edited, saved, "`split` is not in the blob to edit");

    let rc = render_config_from_persist(&edited).expect("a version-floor blob still parses");
    assert!(rc.frame.split.is_finite(), "split opened at {}", rc.frame.split);
    assert!(
        rc.frame.split >= 0.05 && rc.frame.split <= 0.95,
        "split opened at {}, outside its bar's range",
        rc.frame.split,
    );
}
