//! The Tuning pane: the three prime-interval bars, the meantone lock,
//! and the tuning-learn controls.

use super::param_bar;
use crate::params::{ParamBackend, ParamKey};
use crate::widgets::{button_row, ValueBar};
use crate::{theme, SharedState};
use super::learn_pulse;
use lattice_core::tuning;

/// The major-third bar while meantone mode drives it: read-only, showing
/// the derived value (four fifths minus two octaves) the lattice actually
/// uses. Distinct label + dimmed bar make the lock obvious.
fn locked_third_bar(ui: &mut egui::Ui, params: &dyn ParamBackend) {
    let mut derived = tuning::meantone_third(params.get(ParamKey::Three));
    ValueBar::new(&mut derived, ParamKey::Five.range(), "Major third (¢, locked)")
        .decimals(2)
        .locked(true)
        .show(ui)
        .on_hover_text(
            "Meantone: the major third follows the perfect fifth \
             (four fifths minus two octaves)",
        );
}

pub(super) fn tuning_pane(
    ui: &mut egui::Ui,
    state: &mut SharedState,
    params: &dyn ParamBackend,
    now: f64,
) {
    // Tuning sliders. In meantone mode the major third is locked to four
    // perfect fifths, so its bar is shown read-only at the derived value.
    for &key in &ParamKey::TUNING {
        if key == ParamKey::Five && state.view.meantone {
            locked_third_bar(ui, params);
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
            // 12-TET is itself a meantone (400 = 4·700 − 2400), so it's
            // consistent either way; leave the lock as the user has it.
        }
        // Meantone mode: lock the major third to four perfect fifths.
        // Toggle switches, not buttons: Meantone and Learn are persistent
        // modes and must not read like the momentary presets beside them.
        let meantone = crate::widgets::toggle_switch(ui, &mut state.view.meantone, "Meantone")
            .on_hover_text(
                "Lock the major third to four perfect fifths (temper out \
                 the syntonic comma); note-name labels drop their comma marks",
            );
        if meantone.changed() && !state.view.meantone {
            // Turning off: keep the third where the lock left it so the
            // now-editable bar doesn't jump.
            params.set(
                ParamKey::Five,
                tuning::meantone_third(params.get(ParamKey::Three)),
            );
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

    ui.separator();
    // Cross-pane highlight demo: this pane reacts to the lattice hover,
    // reporting the hovered node's pitch class under the current tuning.
    match state.hovered {
        Some(pos) => {
            let pc = state.tuning.pitch_class(pos);
            ui.label(format!(
                "Hovered: ({}, {}, {}) = {}",
                pos.threes, pos.fives, pos.sevens, pc
            ));
        }
        None => {
            ui.weak("Hover a node to inspect it");
        }
    }
}
