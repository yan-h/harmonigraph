//! The Tuning pane: the three prime-interval bars and the tuning-learn
//! controls, then the commas the lattice tempers out, then — under a third
//! heading — the framing controls from [`super::frame`].
//!
//! Three sections in one tab because they are three parts of one question:
//! what the lattice IS. The bars and the commas fix where its nodes sit in
//! pitch (the bars by number, the commas by identity), and framing fixes
//! which of them you are looking at and from where. Everything else in the
//! settings dock is about how what is there gets drawn.

use super::param_bar;
use super::section;
use crate::params::{ParamBackend, ParamKey};
use crate::widgets::{button_row, ValueBar};
use crate::{theme, SharedState};
use super::learn_pulse;
use harmonigraph_core::tuning;

/// The param bar for the axis a comma derives — the one whose tuning is not
/// its own while that comma is tempered out.
///
/// It reads out the DERIVED value (the major third as four fifths, the
/// harmonic seventh as two fifths plus two thirds — the value the lattice
/// actually uses) rather than the param, which is inert while the lock holds;
/// the badge at the front of its name is what says the number is not the
/// param's. (Front, because that is the end a narrow column's elision cannot
/// reach — see `ValueBar::show`.)
///
/// The axes it derives FROM are read off the frame's tuning rather than the
/// params, so a derived value stacks on a derived value: with both commas
/// tempered the seventh follows the third that meantone is deriving, which is
/// what makes the pair read as septimal meantone rather than as two locks
/// disagreeing about the third.
///
/// Draggable all the same: dragging it clear of the derived value is how the
/// mode is let go of. [`tuning::TEMPER_TOLERANCE`] is the width of that
/// clearance, held by the bar's own magnet so a value inside it reads back
/// as the derived one — but at half a cent on an 80¢ bar the magnet is a
/// pixel or two, so in practice any drag you can see releases. It is a
/// release threshold rather than a snap you can feel; what actually snaps
/// TO a temperament is a preset, a learned chord, or the switch.
///
/// Two things the swap to a plain [`param_bar`] on release rests on. The
/// widget id is the same either way — both allocate at the same position in
/// the same loop — so a drag that releases mid-gesture carries on into the
/// bar that replaces it instead of ending on the spot. And the value it then
/// draws is whatever the param reports, which for a frame or more is the
/// value the release is still writing (see `begin_frame` on the plugin's
/// queued writes) — while the lock held, that param was inert and can be
/// anywhere on the bar, so the readout flickers through it on the way.
///
/// A derived value can also be off the bar's ends: the fifth's range is wider
/// than a quarter of the third's, so a fifth outside ~686.6–706.6¢ derives a
/// third the Five range excludes. The readout is still the honest value (the
/// lattice really is using it), the fill saturates, and no drag can reach the
/// magnet — so every drag releases, which is the right answer for a value you
/// cannot get back to anyway.
fn tempered_bar(
    ui: &mut egui::Ui,
    state: &mut SharedState,
    params: &dyn ParamBackend,
    comma: tuning::Comma,
) {
    let key = derived_key(comma);
    let derived = comma.derived(state.tuning.three_cents(), state.tuning.five_cents());
    let mut value = derived;
    let response = ValueBar::new(&mut value, key.range(), key.label())
        .decimals(2)
        .badge(comma.temperament())
        .magnet(derived, tuning::TEMPER_TOLERANCE)
        .show(ui)
        .on_hover_text(format!(
            "{}: the {} follows {}. Drag it more than {}¢ away to release the mode",
            comma.temperament(),
            comma.derived_axis_name(),
            comma.derived_from(),
            tuning::TEMPER_TOLERANCE,
        ));
    // Bracketed like `param_bar`, so a drag that ends in a release records as
    // one host gesture rather than a bare set in the middle of nothing.
    if response.drag_started() {
        params.begin_set(key);
    }
    // A drag inside the window comes back at the derived value and reports no
    // change at all, so this fires only on an edit that escaped. The distance
    // is re-checked for the typed path, which reports every commit as a change
    // whether or not the magnet took the value — and it is the identity's own
    // tolerance, since the value being edited IS the derived axis.
    if response.changed() && (value - derived).abs() > tuning::TEMPER_TOLERANCE {
        *state.view.temper_mut(comma) = false;
        params.set(key, value);
    }
    if response.drag_stopped() {
        params.end_set(key);
    }
}

/// The tuning param a comma derives — where its identity lands, and so which
/// bar goes over to [`tempered_bar`] while it is tempered out.
fn derived_key(comma: tuning::Comma) -> ParamKey {
    match comma {
        tuning::Comma::Syntonic => ParamKey::Five,
        tuning::Comma::SeptimalKleisma => ParamKey::Seven,
    }
}

/// The comma currently deriving this axis, if any. At most one: each comma
/// derives a different axis.
fn comma_deriving(key: ParamKey, view: &harmonigraph_scene::ViewConfig) -> Option<tuning::Comma> {
    tuning::Comma::ALL.into_iter().find(|&c| derived_key(c) == key && view.tempers(c))
}

/// The tempering switches, one row per comma: temper it out, and whether the
/// tuning may engage it by itself.
///
/// Its own section rather than two more switches in the preset row, because
/// there is nothing momentary about them — they are what the lattice IS, and
/// each new comma is another row here rather than another special case.
///
/// Toggle switches, not buttons, for the same reason Learn is one: these are
/// persistent modes and must not read like the presets above them.
fn comma_controls(ui: &mut egui::Ui, state: &mut SharedState) {
    for comma in tuning::Comma::ALL {
        // button_row: the switches are shorter than a padded button, and a
        // plain horizontal row would seat them a few pixels high.
        button_row(ui, |ui| {
            // Nothing is written to the derived param either way. A lock only
            // ever DERIVED that axis; handing the derived value back on the
            // way out would rewrite a tuning the user set (a just third comes
            // back as a tempered one) as a side effect of pressing a mode
            // twice.
            //
            // Live whatever Auto is doing: switching this ON is how a tuning
            // that is NOT within the tolerance gets tempered anyway, and the
            // detect never releases, so that decision stands. Switching it
            // OFF stands too — the detect judges each tuning once, and this
            // one has been judged (see `begin_frame`).
            //
            // Labelled by RATIO, which is the thing a row of these has to
            // tell apart, and short enough to survive a narrow column (see
            // `no_settings_pane_overruns_a_narrow_column`). The temperament
            // it names leads the hover, and is the badge on the bar the lock
            // drives, so the two are never more than a hover apart.
            let auto_on = state.view.temper_auto(comma);
            crate::widgets::toggle_switch(ui, state.view.temper_mut(comma), comma.ratio())
                .on_hover_text(format!(
                    "{}: lock the {} to {}, tempering out the {} ({:.2}¢). Note names \
                     are respelled to match{}",
                    comma.temperament(),
                    comma.derived_axis_name(),
                    comma.derived_from(),
                    comma.comma_name(),
                    comma.size_cents(),
                    if auto_on {
                        ". Auto engages it too, and switching it off here holds until the \
                         tuning changes"
                    } else {
                        ""
                    },
                ));
            // Auto-detect. Switching it ON re-opens the question on the tuning
            // already loaded — without clearing the verdict it would engage
            // nothing until the tuning next moved, since `begin_frame` records
            // every tuning it sees whether the detect is running or not.
            // Switching it off leaves the mode where it is, with the switch
            // beside it still live.
            let auto = crate::widgets::toggle_switch(ui, state.view.temper_auto_mut(comma), "Auto")
                .on_hover_text(format!(
                    "Engage {} by itself whenever the {} lands within {}¢ of {} — from a \
                     preset, a learned chord, or a drag of any bar",
                    comma.temperament(),
                    comma.derived_axis_name(),
                    tuning::TEMPER_TOLERANCE,
                    comma.derived_from(),
                ));
            if auto.changed() && state.view.temper_auto(comma) {
                state.temper_judged[comma.index()] = None;
            }
        });
    }
}

pub(super) fn tuning_pane(
    ui: &mut egui::Ui,
    state: &mut SharedState,
    params: &dyn ParamBackend,
    now: f64,
) {
    // A plain heading rather than `section`: this is the top of the pane, and
    // the leading rule `section` draws would be a line under nothing. Matches
    // the Nodes, Scene, Panel and Analyzer panes.
    ui.heading("Tuning");
    // Tuning sliders. A comma that is tempered out derives one of these axes
    // (meantone the major third, marvel the harmonic seventh), so that axis's
    // bar shows the derived value and is where the mode is released.
    for &key in &ParamKey::TUNING {
        match comma_deriving(key, &state.view) {
            Some(comma) => tempered_bar(ui, state, params, comma),
            None => {
                param_bar(ui, params, key);
            }
        }
    }

    // button_row: the toggle switch is shorter than the padded preset
    // buttons, so a plain horizontal row would seat it a few pixels high.
    button_row(ui, |ui| {
        if ui.button("Just").clicked() {
            params.set(ParamKey::Three, tuning::THREE_JUST);
            params.set(ParamKey::Five, tuning::FIVE_JUST);
            params.set(ParamKey::Seven, tuning::SEVEN_JUST);
            // Just intonation keeps every comma there is, so it is none of
            // these temperaments: drop the locks instead of silently
            // overriding the just tuning we just set.
            for comma in tuning::Comma::ALL {
                *state.view.temper_mut(comma) = false;
            }
        }
        if ui.button("12-TET").clicked() {
            params.set(ParamKey::Three, tuning::THREE_12TET);
            params.set(ParamKey::Five, tuning::FIVE_12TET);
            params.set(ParamKey::Seven, tuning::SEVEN_12TET);
            // 12-TET tempers out both commas (400 = 4·700 − 2400,
            // 1000 = 2·700 + 2·400 − 1200), so the modes are consistent
            // either way; each auto-detect engages its own from the tuning on
            // the next frame, and with a detect off that lock is left as the
            // user has it.
        }
        // v1's tuning-learn mode: while engaged, the tuning re-learns
        // instantly whenever the set of held notes changes (see root_ui).
        let learn = crate::widgets::toggle_switch(ui, &mut state.learn_active, "Learn")
            .on_hover_text("While active, continuously set the tuning from the held notes");
        if state.learn_active {
            // Pulsing armed ring so the engaged mode can't be missed.
            ui.painter().rect_stroke(
                learn.rect.expand(2.0),
                egui::CornerRadius::same(6),
                egui::Stroke::new(2.0, theme::armed().gamma_multiply(learn_pulse(now))),
                egui::StrokeKind::Outside,
            );
        }
    });

    // Which commas the lattice tempers out: the same question as the bars
    // above (what IS this tuning), but the answer is a set of identities
    // rather than three numbers, so it gets its own heading.
    section(ui, "Commas");
    comma_controls(ui, state);

    // Hovering a lattice node deliberately reports NOTHING here. Growing a
    // "Hovered: (t, f, s) = pitch" line whenever the pointer is over a node
    // makes the controls below it jump down and back as the pointer crosses
    // the lattice — a readout in one pane moving another pane's buttons.
    // `state.hovered` drives the lattice's own highlight, which is where a
    // hover belongs.

    // How the lattice is framed: the other half of "what am I looking at".
    section(ui, "Frame");
    super::frame::frame_controls(ui, state);
}
