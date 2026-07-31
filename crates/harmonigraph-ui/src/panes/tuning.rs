//! The Tuning pane: the three prime-interval bars, the meantone lock, and the
//! tuning-learn controls — then, under a second heading, the framing controls
//! from [`super::frame`].
//!
//! Two sections in one tab because they are the two halves of one question:
//! what the lattice IS. Tuning fixes where its nodes sit in pitch, framing
//! fixes which of them you are looking at and from where. Everything else in
//! the settings dock is about how what is there gets drawn.

use super::param_bar;
use super::section;
use crate::params::{ParamBackend, ParamKey};
use crate::widgets::{button_row, ValueBar};
use crate::{theme, SharedState};
use super::learn_pulse;
use harmonigraph_core::tuning;

/// The major-third bar while meantone mode drives it. It reads out the
/// DERIVED third — four fifths minus two octaves, the value the lattice
/// actually uses — rather than the third param, which is inert while the
/// lock holds; the badge at the front of its name is what says the number
/// is not the param's. (Front, because that is the end a narrow column's
/// elision cannot reach — see `ValueBar::show`.)
///
/// Draggable all the same, and with the auto-detect on this is the only way
/// out of the mode. Inside [`tuning::MEANTONE_TOLERANCE`] the drag is
/// swallowed and the bar springs back to the derived value — that magnet is
/// what "snapping to four fifths" means from the pointer's side. Past the
/// tolerance the mode drops and the param takes the dragged value, so the
/// bar carries on from exactly where the pointer left it rather than jumping.
fn meantone_third_bar(ui: &mut egui::Ui, state: &mut SharedState, params: &dyn ParamBackend) {
    let three = params.get(ParamKey::Three);
    let derived = tuning::meantone_third(three);
    let mut value = derived;
    let response = ValueBar::new(&mut value, ParamKey::Five.range(), ParamKey::Five.label())
        .decimals(2)
        .badge("Meantone")
        .magnet(derived, tuning::MEANTONE_TOLERANCE)
        .show(ui)
        .on_hover_text(format!(
            "Meantone: the major third follows the perfect fifth (four fifths \
             minus two octaves). Drag it more than {}¢ away to release the mode",
            tuning::MEANTONE_TOLERANCE,
        ));
    // Bracketed like `param_bar`, so a drag that ends in a release records as
    // one host gesture rather than a bare set in the middle of nothing.
    if response.drag_started() {
        params.begin_set(ParamKey::Five);
    }
    // A drag inside the window comes back at the derived value and reports no
    // change at all, so this fires only on an edit that escaped. The distance
    // is re-checked for the typed path, which reports every commit as a change
    // whether or not the magnet took the value.
    if response.changed() && !tuning::is_meantone(three, value) {
        state.view.meantone = false;
        params.set(ParamKey::Five, value);
    }
    if response.drag_stopped() {
        params.end_set(ParamKey::Five);
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
    // Tuning sliders. In meantone mode the major third is locked to four
    // perfect fifths, so its bar shows the derived value and is the release.
    for &key in &ParamKey::TUNING {
        if key == ParamKey::Five && state.view.meantone {
            meantone_third_bar(ui, state, params);
        } else {
            param_bar(ui, params, key);
        }
    }

    // button_row: the toggle switches are shorter than the padded preset
    // buttons, so a plain horizontal row would seat them a few pixels high.
    button_row(ui, |ui| {
        if ui.button("Just").clicked() {
            params.set(ParamKey::Three, tuning::THREE_JUST);
            params.set(ParamKey::Five, tuning::FIVE_JUST);
            params.set(ParamKey::Seven, tuning::SEVEN_JUST);
            // Just intonation keeps the syntonic comma, so it isn't a
            // meantone: drop the lock instead of silently overriding the
            // just third we just set.
            state.view.meantone = false;
        }
        if ui.button("12-TET").clicked() {
            params.set(ParamKey::Three, tuning::THREE_12TET);
            params.set(ParamKey::Five, tuning::FIVE_12TET);
            params.set(ParamKey::Seven, tuning::SEVEN_12TET);
            // 12-TET is itself a meantone (400 = 4·700 − 2400), so the mode
            // is consistent either way; the auto-detect engages it from the
            // pair on the next frame, and with the detect off the lock is
            // left as the user has it.
        }
        // Meantone mode: lock the major third to four perfect fifths.
        // Toggle switches, not buttons: Meantone, Auto and Learn are
        // persistent modes and must not read like the momentary presets
        // beside them.
        //
        // Live whatever Auto is doing: switching it ON is how a tuning that
        // is NOT within the tolerance gets snapped to meantone anyway, and
        // the detect never releases, so that decision stands. Switching it
        // OFF stands too — the detect judges each tuning once, and this one
        // has been judged (see `begin_frame`).
        //
        // Nothing is written to the third param either way. The lock only
        // ever DERIVED the third; handing that derived value back on the way
        // out would rewrite a tuning the user set (a just third comes back as
        // a tempered one) as a side effect of pressing a mode twice.
        crate::widgets::toggle_switch(ui, &mut state.view.meantone, "Meantone")
            .on_hover_text(if state.view.meantone_auto {
                "Lock the major third to four perfect fifths (temper out the \
                 syntonic comma); note-name labels drop their comma marks. Auto \
                 engages it too, and switching it off here holds until the \
                 tuning changes"
            } else {
                "Lock the major third to four perfect fifths (temper out the \
                 syntonic comma); note-name labels drop their comma marks"
            });
        // Auto-detect. Nothing to do on a change: switched on, the detect
        // runs in `begin_frame` and engages from the tuning itself; switched
        // off, the mode simply stays where it is with the switch live again.
        crate::widgets::toggle_switch(ui, &mut state.view.meantone_auto, "Auto").on_hover_text(
            format!(
                "Engage meantone by itself whenever the major third lands within \
                 {}¢ of four perfect fifths — from a preset, a learned chord, or \
                 a drag of either bar",
                tuning::MEANTONE_TOLERANCE,
            ),
        );
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
