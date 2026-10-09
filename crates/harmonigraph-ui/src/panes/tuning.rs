//! The Tuning pane: interval bars with their temperament links, Learn and note matching, then how incoming notes are retuned (Pass
//! through, Adaptive or a Lattice Map, each with its own controls) and the
//! tuning instances.
//!
//! The bars and the commas share a tab because they are two halves of one
//! question: where the lattice's nodes sit in pitch, the bars answering it by
//! number and the commas by identity. Which of those nodes you are then
//! looking at is [`super::view`], and it is a tab rather than a third section
//! here because a tab called Tuning is not where anyone looks for a camera.
//! Everything else in the settings dock is about how what is there gets drawn.

use super::learn_pulse;
use super::param_bar;
use super::spectral::roll::note_color;
use super::{section, subsection};
use crate::params::{ParamBackend, ParamKey};
use crate::widgets::{button_row, ValueBar};
use crate::{theme, PictureState};
use harmonigraph_core::configuration::{ConfigEdit, PolicyEdit};
use harmonigraph_core::tuning;
use std::collections::BTreeMap;

#[cfg(test)]
#[path = "tuning_instances_tests.rs"]
mod instance_tests;

#[cfg(test)]
#[path = "tuning_map_tests.rs"]
mod map_tests;

/// Configuration status bit 2: the audio owner refuses every edit and fails a
/// recording's configuration until the host resets the plugin.
pub(crate) const CONFIGURATION_FAULT_NOTICE: &str =
    "Tuning changes refused: configuration fault. Deactivate and reactivate the plugin in the host to recover.";
/// Configuration status bit 1: the held notes cannot be read for Learn.
pub(crate) const HELD_STATE_INCOMPLETE_NOTICE: &str =
    "Learning unavailable: held state is incomplete. Reset to recover.";

/// Interval entry always submits a semantic edit, even when its typed value is
/// unchanged. Linked readouts use the resolved value; their ordinary edit path
/// recognizes or releases the relationship in the configuration owner.
fn interval_bar(
    ui: &mut egui::Ui,
    state: &mut PictureState,
    params: &dyn ParamBackend,
    key: ParamKey,
) {
    let current = live_tuning(state, params);
    let mut value = match key {
        ParamKey::Three => current.three_cents(),
        ParamKey::Five => current.five_cents(),
        ParamKey::Seven => current.seven_cents(),
        _ => unreachable!(),
    };
    let linked = comma_deriving(key, &state.appearance.view);
    let mut bar = ValueBar::new(&mut value, key.range(), key.label()).decimals(2).unit(1.0, "¢");
    if linked.is_some() {
        bar = bar.badge("↔");
    }
    let response = bar.show(ui).on_hover_text(match linked {
        Some(comma) => format!(
            "{}: the {} follows {}. Enter an independent value to change the relationship.",
            comma.temperament(),
            comma.derived_axis_name(),
            comma.derived_from()
        ),
        None => tuning_hint(key).to_owned(),
    });
    if response.drag_started() {
        params.begin_set(key);
    }
    if response.changed() {
        let index = ParamKey::TUNING.iter().position(|&k| k == key).unwrap();
        state.runtime.edit_tuning(
            &mut state.appearance,
            params,
            ConfigEdit::axis(index, tuning::microcents(value)),
        );
    }
    if response.drag_stopped() {
        params.end_set(key);
    }
}

fn temperament_switch(
    ui: &mut egui::Ui,
    state: &mut PictureState,
    params: &dyn ParamBackend,
    comma: tuning::Comma,
) {
    let mut on = state.appearance.view.tempers(comma);
    let response = crate::widgets::toggle_switch(ui, &mut on, comma.temperament()).on_hover_text(format!(
        "Link the {} to {} and give equivalent notes the same name. Switching off keeps the current interval. New tuning entries and Learn recognize this relationship automatically.",
        comma.derived_axis_name(), comma.derived_from(),
    ));
    if response.changed() {
        let edit = ConfigEdit::temper(comma, on, live_tuning(state, params));
        state.runtime.edit_tuning(&mut state.appearance, params, edit);
    }
}

/// Keep each switch beside its interval when there is room. In a narrow pane
/// it wraps below that interval, so neither the entry nor its switch is clipped.
fn interval_row(
    ui: &mut egui::Ui,
    state: &mut PictureState,
    params: &dyn ParamBackend,
    comma: tuning::Comma,
) {
    let key = derived_key(comma);
    let switch_width = tuning::Comma::ALL
        .into_iter()
        .map(|c| crate::widgets::toggle_switch_width(ui, c.temperament()))
        .fold(0.0, f32::max);
    let bar_width = ui.available_width() - switch_width - ui.spacing().item_spacing.x;
    if bar_width >= 170.0 * theme::ui_scale(ui.ctx()) {
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(bar_width, theme::row_height(theme::ui_scale(ui.ctx()))),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    interval_bar(ui, state, params, key);
                },
            );
            temperament_switch(ui, state, params, comma);
        });
    } else {
        interval_bar(ui, state, params, key);
        button_row(ui, |ui| temperament_switch(ui, state, params, comma));
    }
}

/// The tuning as the lattice will use it, from the params as they stand right
/// now: `begin_frame`'s derivation, re-run on live values.
///
/// The live read is what keeps a derived bar current with the bar it follows.
/// `ParamKey::TUNING` draws the fifth before the third and the third before
/// the seventh, and in a shell whose `set` lands immediately (the standalone)
/// a drag of the fifth is already in the params by the time the bars below it
/// draw — reading the frame's snapshot instead would leave every derived
/// readout a frame behind for the whole gesture.
fn live_tuning(state: &PictureState, params: &dyn ParamBackend) -> harmonigraph_core::Tuning {
    if let Some(owned) = params.configuration() {
        return owned.resolved.tuning;
    }
    let mut tuning = crate::params::tuning_from_params(params);
    for comma in tuning::Comma::ALL {
        if state.appearance.view.tempers(comma) {
            tuning.temper(comma);
        }
    }
    tuning
}

/// The interval whose tuning and inline switch this comma controls.
fn derived_key(comma: tuning::Comma) -> ParamKey {
    match comma {
        tuning::Comma::Syntonic => ParamKey::Five,
        tuning::Comma::SeptimalKleisma => ParamKey::Seven,
    }
}

/// Unlinked interval hints. [`interval_bar`] explains the active relationship
/// instead while an interval is linked.
fn tuning_hint(key: ParamKey) -> &'static str {
    match key {
        ParamKey::COffset => {
            "Where C sits, in cents from standard (100¢ is a semitone). Moves \
             every node's pitch together."
        }
        ParamKey::Three => {
            "The fifths axis: one step, in cents. 701.96 is just (3:2), 700 is \
             12-TET."
        }
        ParamKey::Five => {
            "The thirds axis: one step, in cents. 386.31 is just (5:4), 400 is \
             12-TET. Tempering Meantone locks it to the fifth."
        }
        ParamKey::Seven => {
            "The sevenths axis: one step, in cents. 968.83 is just (7:4), 1000 \
             is 12-TET. Tempering Marvel locks it to the fifth and third."
        }
        ParamKey::Tolerance => {
            "How far off a node's pitch a note may land and still light it, in \
             cents. Also sets the Analyzer's off-lattice band."
        }
        // Not on this pane: Fade is a node setting and the two pitch ends are
        // the Mappings page's Pitch color range.
        ParamKey::Fade
        | ParamKey::DarkestPitch
        | ParamKey::BrightestPitch
        | ParamKey::CameraYaw
        | ParamKey::CameraPitch
        | ParamKey::CameraDistance
        | ParamKey::CameraPanX
        | ParamKey::CameraPanY => "",
    }
}

/// The comma currently deriving this axis, if any. At most one: each comma
/// derives a different axis.
fn comma_deriving(key: ParamKey, view: &harmonigraph_scene::ViewConfig) -> Option<tuning::Comma> {
    tuning::Comma::ALL.into_iter().find(|&c| derived_key(c) == key && view.tempers(c))
}

pub(super) fn tuning_pane(
    ui: &mut egui::Ui,
    state: &mut PictureState,
    params: &dyn ParamBackend,
    now: f64,
) {
    section(ui, "Tuning", |ui| {
        param_bar(ui, params, ParamKey::COffset).on_hover_text(tuning_hint(ParamKey::COffset));
        interval_bar(ui, state, params, ParamKey::Three);
        for comma in tuning::Comma::ALL {
            interval_row(ui, state, params, comma);
        }

        button_row(ui, |ui| {
            if ui
                .button("Just")
                .on_hover_text(
                    "Pure ratios on every axis — 3:2, 5:4, 7:4 — and both \
                     temperaments released.",
                )
                .clicked()
            {
                state.runtime.edit_tuning(
                    &mut state.appearance,
                    params,
                    ConfigEdit {
                        axes: [
                            None,
                            Some(tuning::microcents(tuning::THREE_JUST)),
                            Some(tuning::microcents(tuning::FIVE_JUST)),
                            Some(tuning::microcents(tuning::SEVEN_JUST)),
                            None,
                        ],
                        ..Default::default()
                    },
                );
            }
            if ui
                .button("12-TET")
                .on_hover_text(
                    "Equal-tempered steps — 700, 400, 1000 cents. Matches a plain \
                     MIDI keyboard.",
                )
                .clicked()
            {
                state.runtime.edit_tuning(
                    &mut state.appearance,
                    params,
                    ConfigEdit {
                        axes: [
                            None,
                            Some(tuning::microcents(tuning::THREE_12TET)),
                            Some(tuning::microcents(tuning::FIVE_12TET)),
                            Some(tuning::microcents(tuning::SEVEN_12TET)),
                            None,
                        ],
                        ..Default::default()
                    },
                );
            }
            // Learn: while engaged, the tuning re-learns whenever the set of
            // held notes changes. The plugin's audio owner does it
            // (`Owner::group_end`); without one, `VisualRuntime::learn_step`.
            let mut learn_active = state.runtime.learn_active;
            let learn = crate::widgets::toggle_switch(ui, &mut learn_active, "Learn")
                .on_hover_text(
                    "While active, set the tuning from the held notes whenever they change. While a source has Retune on, only the C offset and the keyboard are learned: the lattice axes are what it is retuned to.",
                );
            if learn.changed() {
                state.runtime.edit_tuning(
                    &mut state.appearance,
                    params,
                    ConfigEdit { learning: Some(learn_active), ..Default::default() },
                );
            }
            if state.runtime.learn_active {
                // Pulsing armed ring so the engaged mode can't be missed. It
                // stands `pad` out from the switch, so its corners are the
                // switch's radius plus `pad` and the two round about one centre.
                let scale = theme::ui_scale(ui.ctx());
                let pad = theme::scaled_points(2, scale);
                ui.painter().rect_stroke(
                    learn.rect.expand(f32::from(pad)),
                    egui::CornerRadius::same(theme::control_radius(scale) + pad),
                    egui::Stroke::new(2.0, theme::armed().gamma_multiply(learn_pulse(now))),
                    egui::StrokeKind::Outside,
                );
            }
        });
    });

    // Faults remain visible even when the tuning controls are folded.
    let configuration_notice = if state.runtime.configuration_status & 2 != 0 {
        crate::widgets::label(
            ui,
            egui::RichText::new(CONFIGURATION_FAULT_NOTICE).color(theme::armed()),
        );
        true
    } else if state.runtime.configuration_status & 1 != 0 {
        crate::widgets::label(
            ui,
            egui::RichText::new(HELD_STATE_INCOMPLETE_NOTICE).color(theme::armed()),
        );
        true
    } else if state.runtime.configuration_status & 4 != 0 {
        crate::widgets::weak(ui, "Tuning applied; host notification was rejected");
        true
    } else if state.runtime.configuration_status & 8 != 0 {
        crate::widgets::weak(ui, "Tuning change refused: pending command storage is full");
        true
    } else {
        false
    };

    section(ui, "Note matching", |ui| {
        param_bar(ui, params, ParamKey::Tolerance).on_hover_text(tuning_hint(ParamKey::Tolerance));
    });

    let mode = map_controls(ui, state, params);
    if mode == harmonigraph_core::lattice_map::TuningEngine::Adaptive {
        adaptive_controls(ui, state, params);
    }
    instance_section(ui, state, params);
    // Keep transient status after every control so it cannot move a held slider.
    if !configuration_notice && state.runtime.configuration_pending {
        crate::widgets::weak(ui, "Tuning change pending audio adoption");
    }

    // Hovering a lattice node deliberately reports NOTHING here. Growing a
    // "Hovered: (t, f, s) = pitch" line whenever the pointer is over a node
    // makes the controls below it jump down and back as the pointer crosses
    // the lattice — a readout in one pane moving another pane's buttons.
    // `state.surfaces.hovered` drives the lattice's own highlight, which is where a
    // hover belongs.
}

/// How the player's controller renders each prime. Learn fills it from the
/// fifth it hears; it is an adaptive policy setting, so edits travel as one.
fn keyboard_controls(ui: &mut egui::Ui, state: &mut PictureState, params: &dyn ParamBackend) {
    let mut p = state.runtime.adaptive_policy;
    let before = p;
    let mut derive_keyboard = false;
    subsection(ui, "Keyboard", |ui| {
        for (value, label) in p.keyboard.iter_mut().zip(["Fifth", "Third", "Seventh"]) {
            *value =
                adaptive_value(ui, *value as u32, 0..=1_200_000_000, 1_000_000.0, label, "¢", "")
                    as i32;
        }
        let [third, seventh] = tuning::fifth_generated_steps(p.keyboard[0]).map(fifths);
        if ui
            .button("Derive from fifth")
            .on_hover_text(format!(
                "Set the input keyboard's third and seventh from its fifth: {third} and {seventh}."
            ))
            .clicked()
        {
            derive_keyboard = true;
        }
    })
    .on_hover_text(
        "Tuning of the incoming keyboard. A key may only become a lattice node this keyboard would play at the pitch the key \
         sent; when no key matches, all local nodes compete. A note stays unsnapped when \
         pitch cost outweighs harmonic benefit. Learn sets the keyboard from the \
         fifth it hears.",
    );
    let mut policy = PolicyEdit::changed(before, p);
    if derive_keyboard {
        policy = policy.derive_keyboard();
    }
    if !policy.is_empty() {
        state.runtime.edit_tuning(
            &mut state.appearance,
            params,
            ConfigEdit { policy: Some(policy), ..Default::default() },
        );
    }
}

fn fifths(steps: i32) -> String {
    let n = steps.unsigned_abs();
    let plural = if n == 1 { "" } else { "s" };
    format!("{n} fifth{plural} {}", if steps < 0 { "down" } else { "up" })
}

fn adaptive_controls(ui: &mut egui::Ui, state: &mut PictureState, params: &dyn ParamBackend) {
    section(ui, "Adaptive tuning", |ui| {
        keyboard_controls(ui, state, params);
        let mut p = state.runtime.adaptive_policy;
        let before = p;
        p.pitch_flexibility = adaptive_value(
            ui, p.pitch_flexibility.into(), 1..=100, 1.0, "Pitch flexibility", "¢",
            "Cents of displacement beyond accumulated drift that cost one point. The exponential penalty rises increasingly quickly; a note stays unsnapped when its harmonic benefit cannot cover that cost.",
        ) as u16;
        p.radius =
            adaptive_value(ui, p.radius.into(), 1..=5, 1.0, "Search radius", " steps", "Candidate distance along each enabled lattice axis. Larger radii consider more tuning alternatives and cost more processing.").max(1) as u8;
        crate::widgets::label(ui, "Search axes").on_hover_text(
            "Lattice axes the adaptive tuner may use when looking for a note's candidate pitches.",
        );
        theme::reserve_scroll_gutter(ui);
        egui::ScrollArea::horizontal().id_salt("adaptive-axes-scroll").show(ui, |ui| {
            crate::widgets::selected_combo(
                ui,
                egui::ComboBox::from_id_salt("adaptive-axes").selected_text(match p.axes {
                    1 => "Fifths",
                    2 => "Fifths + thirds",
                    _ => "Fifths + thirds + sevenths",
                }),
                |ui| {
                    ui.selectable_value(&mut p.axes, 1, "Fifths");
                    ui.selectable_value(&mut p.axes, 2, "Fifths + thirds");
                    ui.selectable_value(&mut p.axes, 3, "Fifths + thirds + sevenths");
                },
            );
        });
        let context = subsection(ui, "Context", |ui| {
            p.half_life_ms = adaptive_value(
                ui,
                p.half_life_ms.into(),
                0..=20_000,
                1000.0,
                "Half-life",
                "s",
                "A note struck this long before the newest counts half. 0: no decay.",
            ) as u16;
            p.register = adaptive_value(
                ui,
                p.register.into(),
                10..=1000,
                1000.0,
                "Weight per octave",
                "",
                "Each octave between two notes multiplies the vote by this. 1: ignore register.",
            ) as u16;
            p.tolerance = adaptive_value(
                ui,
                p.tolerance,
                0..=20_000_000,
                1_000_000.0,
                "Same-note tolerance",
                "¢",
                "Onsets this close in pitch count as one note.",
            );
            p.silence_ms = adaptive_value(
                ui,
                p.silence_ms,
                0..=120_000,
                1000.0,
                "Silence reset",
                "s",
                "Forget the context after this long with nothing held. 0: never.",
            );
            crate::widgets::checkbox(ui, &mut p.reset_stop, "Reset context on stop")
                .on_hover_text("Forget the context when the transport stops.");
            crate::widgets::checkbox(ui, &mut p.reset_loop, "Reset context on loop / seek")
                .on_hover_text("Forget the context when playback jumps.");
        });
        context.on_hover_text(
            "New notes follow the moving context; sounding notes keep their correction.",
        );
        if p != before {
            state.runtime.edit_tuning(
                &mut state.appearance,
                params,
                ConfigEdit { policy: Some(PolicyEdit::changed(before, p)), ..Default::default() },
            );
        }
    });
}

fn instance_section(ui: &mut egui::Ui, state: &PictureState, params: &dyn ParamBackend) {
    let instances = params.tuning_instances();
    if !instances.is_empty() {
        section(ui, "Tuning sources", |ui| instance_controls(ui, state, params, &instances));
    }
}

/// UI-only levels, keyed by MIDI channel/key rather than pitch or packed-list
/// position, so bends and another voice ending cannot restart a dot's fade.
#[derive(Clone, Default)]
struct SourceDots {
    at: Option<f64>,
    dots: BTreeMap<u16, VoiceDot>,
}

#[derive(Clone, Copy)]
struct VoiceDot {
    pitch: f32,
    level: f32,
}

impl SourceDots {
    fn step(
        &mut self,
        voices: &[crate::params::TuningVoice],
        now: f64,
        env: &harmonigraph_core::Envelope,
    ) -> bool {
        let dt = self.at.map_or(0.0, |at| (now - at).max(0.0));
        self.at = Some(now);
        for voice in voices {
            self.dots
                .entry(voice.key)
                .or_insert(VoiceDot { pitch: voice.pitch, level: 0.0 })
                .pitch = voice.pitch;
        }
        let mut animating = false;
        self.dots.retain(|key, dot| {
            let held = voices.iter().any(|voice| voice.key == *key);
            dot.level = env.carried(dot.level, dt, held);
            animating |= dot.level != if held { 1.0 } else { 0.0 };
            held || dot.level > 0.0
        });
        animating
    }
}

/// A single, fixed-height strip; overflowing voices replace its last dot with an ellipsis.
fn instance_voice_dots(ui: &egui::Ui, rect: egui::Rect, voices: &[VoiceDot], state: &PictureState) {
    let scale = theme::ui_scale(ui.ctx());
    let inner = rect.shrink2(egui::vec2(4.0 * scale, 0.0));
    let pitch = 5.0 * scale;
    let capacity = (inner.width().max(0.0) / pitch).floor();
    if capacity == 0.0 {
        return;
    }
    // Capacity follows animated width, not voice count. Otherwise pruning a
    // zero-width tail can promote a fully bright hidden voice in one frame.
    let total = voices.iter().map(|dot| dot.level).sum::<f32>();
    let overflow_level = (total - capacity).clamp(0.0, 1.0);
    let mut remaining = capacity - overflow_level;
    let visible: Vec<_> = voices
        .iter()
        .filter_map(|dot| {
            let level = dot.level.min(remaining);
            remaining -= level;
            (level > 0.0).then_some(VoiceDot { level, ..*dot })
        })
        .collect();
    // Center the painted group, easing the wider ellipsis in with its opacity.
    let last_radius = (1.25 + 1.05 * overflow_level) * scale;
    let first_radius = if capacity == 1.0 { last_radius } else { 1.25 * scale };
    let width = (visible.iter().map(|dot| dot.level).sum::<f32>() + overflow_level) * pitch;
    let mut x = rect.center().x - (width + last_radius - first_radius) * 0.5;
    for dot in visible {
        let width = dot.level * pitch;
        ui.painter().circle_filled(
            egui::pos2(x + width * 0.5, rect.center().y),
            1.25 * scale,
            note_color(state, dot.pitch, dot.level),
        );
        x += width;
    }
    if overflow_level > 0.0 {
        let center = egui::pos2(x + overflow_level * pitch * 0.5, rect.center().y);
        for offset in [-1.7, 0.0, 1.7] {
            ui.painter().circle_filled(
                center + egui::vec2(offset * scale, 0.0),
                0.6 * scale,
                ui.visuals().weak_text_color().gamma_multiply(overflow_level),
            );
        }
    }
}

/// What a source's Retune and Show boxes answer a hover with, the bulk
/// boxes included: off, Retune passes notes through without influencing
/// tuning, and Show only ever affects the picture.
const RETUNE_HINT: &str =
    "Tune new notes with the selected engine. Off, notes pass through without influencing tuning.";
const SHOW_HINT: &str = "Show this instance's output notes. Only affects the picture.";

fn instance_controls(
    ui: &mut egui::Ui,
    state: &PictureState,
    params: &dyn ParamBackend,
    instances: &[crate::params::TuningInstance],
) {
    use crate::params::InstanceEdit;
    let scale = theme::ui_scale(ui.ctx());
    let bulk_control = |ui: &mut egui::Ui, label: &str, retune: bool| {
        let enabled =
            instances.iter().filter(|row| if retune { row.retune } else { row.show }).count();
        let mut all = enabled == instances.len();
        let mixed = enabled != 0 && !all;
        if crate::widgets::checkbox_indeterminate(ui, &mut all, label, mixed)
            .on_hover_text(if retune { RETUNE_HINT } else { SHOW_HINT })
            .clicked()
        {
            let value = enabled != instances.len();
            for row in instances {
                params.edit_tuning_instance(
                    row.id,
                    if retune { InstanceEdit::Retune(value) } else { InstanceEdit::Show(value) },
                );
            }
        }
    };
    let selection = ui.id().with("tuning-instance-selection");
    let mut selected = ui.data(|data| data.get_temp::<u64>(selection)).unwrap_or(instances[0].id);
    if !instances.iter().any(|row| row.id == selected) {
        selected = instances[0].id;
    }
    let name_width = (ui.available_width() - 140.0 * scale).max(60.0 * scale);
    let animation_id = selection.with("voice-dots");
    let mut sources = ui
        .data(|data| data.get_temp::<BTreeMap<u64, SourceDots>>(animation_id))
        .unwrap_or_default();
    sources.retain(|id, _| instances.iter().any(|row| row.id == *id));
    let now = ui.input(|input| input.time);
    let env = state.appearance.view.envelope(&state.runtime.frame_params);
    for row in instances {
        if sources.entry(row.id).or_default().step(&row.voices, now, &env) {
            ui.ctx().request_repaint();
        }
    }
    let mut identity = |ui: &mut egui::Ui, row: &crate::params::TuningInstance| {
        let name = egui::RichText::new(&row.display_name);
        let name = if row.status != "No faults" { name.color(theme::armed()) } else { name };
        let galley = egui::WidgetText::from(name).into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            (name_width - 2.0 * ui.spacing().button_padding.x).max(0.0),
            egui::TextStyle::Button,
        );
        let ink = if galley.mesh_bounds.is_finite() {
            galley.mesh_bounds
        } else {
            egui::Rect::from_min_size(egui::Pos2::ZERO, galley.size())
        };
        // Visible geometry: 3px top + text ink + 2px gap + 2.5px dots + 3px bottom.
        let response = ui.add_sized(
            [name_width, ink.height() + 10.5 * scale],
            egui::Button::selectable(selected == row.id, ()).small(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Button,
                ui.is_enabled(),
                selected == row.id,
                &row.display_name,
            )
        });
        if response.clicked() {
            selected = row.id;
        }
        let ink_top = response.rect.top() + 3.0 * scale;
        let text_pos =
            egui::pos2(response.rect.center().x - galley.size().x * 0.5, ink_top - ink.top());
        ui.painter().galley(
            text_pos,
            galley,
            ui.style().interact_selectable(&response, selected == row.id).text_color(),
        );
        let dots = egui::Rect::from_min_size(
            egui::pos2(response.rect.left(), ink_top + ink.height() + 1.25 * scale),
            egui::vec2(response.rect.width(), 4.0 * scale),
        );
        let voices: Vec<_> = sources[&row.id].dots.values().copied().collect();
        instance_voice_dots(ui, dots, &voices, state);
        let held = row.voices.len();
        response.on_hover_text(format!(
            "{held} sounding {}\n{}",
            if held == 1 { "voice" } else { "voices" },
            row.status,
        ));
    };
    let flags = |ui: &mut egui::Ui, row: &crate::params::TuningInstance| {
        let mut retune = row.retune;
        if crate::widgets::checkbox(ui, &mut retune, "").on_hover_text(RETUNE_HINT).changed() {
            params.edit_tuning_instance(row.id, InstanceEdit::Retune(retune));
        }
        let mut show = row.show;
        if crate::widgets::checkbox(ui, &mut show, "").on_hover_text(SHOW_HINT).changed() {
            params.edit_tuning_instance(row.id, InstanceEdit::Show(show));
        }
    };
    egui::Grid::new("tuning-instances")
        .num_columns(3)
        .min_row_height(0.0)
        .spacing(ui.spacing().item_spacing)
        .show(ui, |ui| {
            crate::widgets::weak(ui, "Source");
            bulk_control(ui, "Retune", true);
            bulk_control(ui, "Show", false);
            ui.end_row();
            for row in instances {
                ui.push_id(row.id, |ui| identity(ui, row));
                flags(ui, row);
                ui.end_row();
            }
        });
    ui.data_mut(|data| {
        data.insert_temp(selection, selected);
        data.insert_temp(animation_id, sources);
    });
    if let Some(row) = instances.iter().find(|row| row.id == selected) {
        ui.push_id(row.id, |ui| {
            subsection(ui, "Source details", |ui| {
                let mut name = row.name.clone();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut name)
                            .hint_text(&row.display_name)
                            .desired_width(ui.available_width()),
                    )
                    .on_hover_text("Custom name; leave empty to follow the track name")
                    .changed()
                {
                    params.edit_tuning_instance(row.id, InstanceEdit::Name(name));
                }
                crate::widgets::label(ui, &row.status);
                crate::widgets::label(
                    ui,
                    egui::RichText::new(format!(
                        "{} notes in · {} out · {} missed corrections",
                        row.notes_in, row.notes_out, row.misses
                    ))
                    .small(),
                );
                if let Some((input, output)) = row.last_pitch {
                    crate::widgets::label(
                        ui,
                        egui::RichText::new(format!(
                            "Last attack: {} → {}",
                            monitor_pitch(input),
                            monitor_pitch(output)
                        ))
                        .small(),
                    )
                    .on_hover_text("Note pitch and per-note expression, before channel pitch bend");
                    crate::widgets::label(
                        ui,
                        egui::RichText::new(format!("Correction: {:+.1}¢", output - input)).small(),
                    );
                }
                let delay_text = crate::widgets::label(ui, &row.delay_text);
                if row.max_delay > 1 {
                    let id = ui.id().with("delay-draft");
                    let mut delay = ui.data(|data| data.get_temp::<u32>(id)).unwrap_or(row.delay);
                    ui.horizontal_wrapped(|ui| {
                        crate::widgets::label(ui, "Buffers of delay");
                        let response =
                            ui.add(egui::DragValue::new(&mut delay).range(1..=row.max_delay));
                        // Commit one completed drag, preserving the draft between frames.
                        let draft = ui.data(|data| data.get_temp::<u32>(id));
                        if response.changed() {
                            ui.data_mut(|data| data.insert_temp(id, delay));
                        }
                        if response.drag_stopped() || response.changed() && !response.dragged() {
                            params.edit_tuning_instance(
                                row.id,
                                InstanceEdit::Delay(if response.changed() {
                                    delay
                                } else {
                                    draft.unwrap_or(delay)
                                }),
                            );
                            ui.data_mut(|data| data.remove::<u32>(id));
                        }
                    });
                } else {
                    delay_text.on_hover_text("Harmonigraph's own input needs one buffer.");
                }
            });
        });
    }
    if let Some(tuner) = instances.iter().find(|row| !row.is_hub) {
        subsection(ui, "Tuning delay", |ui| {
            let id = ui.id().with("all-delay-draft");
            let mut delay = ui.data(|data| data.get_temp::<u32>(id)).unwrap_or(tuner.delay);
            ui.horizontal_wrapped(|ui| {
                crate::widgets::label(ui, "Buffers");
                ui.add(egui::DragValue::new(&mut delay).range(1..=tuner.max_delay));
                if ui
                    .button("Apply to all tuners")
                    .on_hover_text(
                        "Each tuner reports its own latency; Source details overrides one. \
                         Harmonigraph's own input stays at one buffer.",
                    )
                    .clicked()
                {
                    for row in instances {
                        if !row.is_hub {
                            params.edit_tuning_instance(row.id, InstanceEdit::Delay(delay));
                        }
                    }
                }
            });
            ui.data_mut(|data| data.insert_temp(id, delay));
        });
    }
    if ui
        .button("Reset all voices")
        .on_hover_text("Release held notes and clear adaptive tuning context on every instance")
        .clicked()
    {
        params.edit_tuning_instance(instances[0].id, InstanceEdit::Reset);
    }
}

fn monitor_pitch(cents: f64) -> String {
    let midi = (cents / 100.0).round() as i64;
    let name = ["C", "C♯", "D", "E♭", "E", "F", "F♯", "G", "A♭", "A", "B♭", "B"]
        [midi.rem_euclid(12) as usize];
    let offset = cents - midi as f64 * 100.0;
    format!("{name}{} {offset:+.1}¢", midi.div_euclid(12) - 1)
}

/// Use the dock's width-aware value bars so long labels elide rather than widening the pane.
fn adaptive_value(
    ui: &mut egui::Ui,
    raw: u32,
    range: std::ops::RangeInclusive<u32>,
    scale: f32,
    label: &str,
    unit: &str,
    hint: &str,
) -> u32 {
    let mut value = raw as f32 / scale;
    let mut bar = ValueBar::new(
        &mut value,
        *range.start() as f32 / scale..=*range.end() as f32 / scale,
        label,
    )
    .unit(1.0, unit);
    bar = if scale == 1.0 { bar.integer() } else { bar.decimals(2) };
    let response = bar.show(ui);
    let response = if hint.is_empty() { response } else { response.on_hover_text(hint) };
    if response.changed() {
        (value * scale).round() as u32
    } else {
        raw
    }
}

fn map_controls(
    ui: &mut egui::Ui,
    state: &mut PictureState,
    params: &dyn ParamBackend,
) -> harmonigraph_core::lattice_map::TuningEngine {
    use crate::lattice_maps::{
        MapAxis, MapEdit, MapOffsetLane, EXTENSION_STEP, MIDI_LABELS, OFFSET_LIMIT,
    };
    use harmonigraph_core::lattice_map::{Follow, TuningEngine};
    let Some(view) = params.lattice_maps() else {
        return TuningEngine::Off;
    };
    // Stored first and read back by reference: this pane and the rest of the
    // editor want the same view.
    state.runtime.lattice_maps = Some(view);
    let view = state.runtime.lattice_maps.as_ref().expect("just stored");
    section(ui, "Note retuning", |ui| {
        let mut mode = view.playback.engine;
        let before = mode;
        crate::widgets::choice_buttons(
            ui,
            "retuning engine",
            &mut mode,
            &[
                (
                    TuningEngine::Off,
                    "Pass through",
                    "Leave incoming note pitches unchanged. Lattice display tuning still applies.",
                ),
                (
                    TuningEngine::Adaptive,
                    "Adaptive",
                    "Choose tuning for new notes from the musical context.",
                ),
                (
                    TuningEngine::LatticeMap,
                    "Lattice Map",
                    "Tune new notes using the selected saved lattice map.",
                ),
            ],
        );
        if mode != before {
            params.edit_lattice_map(MapEdit::Engine(mode));
        }
        if view.pending {
            crate::widgets::weak(ui, "Map state pending audio adoption");
        }
        if mode != TuningEngine::LatticeMap {
            return mode;
        }
        if state.runtime.learn_active {
            crate::widgets::label(ui, egui::RichText::new("Learn is suspended in Lattice Map.").color(theme::armed()));
        }
        crate::widgets::checkbox(ui, &mut state.appearance.view.show_map_indicators, "Show map indicators")
            .on_hover_text(
                "Show a bright dot above each destination in the selected map. This \
                 does not change the map or its tuning.",
            );
        let selected = view.playback.selected;
        let name = view
            .names
            .iter()
            .find(|(id, _)| *id == selected)
            .map(|(_, name)| &**name)
            .unwrap_or("unavailable");
        crate::widgets::label(ui, "Saved map");
        crate::widgets::selected_combo(
            ui,
            egui::ComboBox::from_id_salt("saved-lattice-map")
                .selected_text(format!("{} · {name}", selected + 1)),
            |ui| {
                for (id, name) in view.names.iter() {
                    if ui.selectable_label(selected == *id, format!("{} · {name}", id + 1)).clicked() {
                        params.edit_lattice_map(MapEdit::Select(*id));
                    }
                }
            },
        )
        .response
        .on_hover_text(
            "The map new notes are tuned by. Map changes affect new attacks; held notes keep \
             their onset tuning.",
        );
        if view.playback.map.is_none() {
            crate::widgets::label(ui, egui::RichText::new("Map unavailable: new attacks pass through.").color(theme::armed()));
        }
        crate::widgets::label(ui, "Follow harmony");
        let mut follow = view.playback.follow;
        crate::widgets::choice_buttons(
            ui,
            "map follow",
            &mut follow,
            &[
                (Follow::Off, "Off", "The map stays where its offsets put it."),
                (
                    Follow::Thirds,
                    "Thirds",
                    "Move the map a step along thirds when that spells a chord more simply, \
                     such as Ab major taking C as a major third rather than a diminished fourth.",
                ),
                (
                    Follow::ThirdsAndFifths,
                    "Thirds and fifths",
                    "Also step along fifths when that clearly helps, such as D F A taking a \
                     10/9 D instead of a wolf fifth. Held notes keep their tuning, so the map \
                     can drift by a comma when a held note forces it.",
                ),
            ],
        );
        if follow != view.playback.follow {
            params.edit_lattice_map(MapEdit::Follow(follow));
        }
        if view.followed != harmonigraph_core::LatticePos::ORIGIN {
            crate::widgets::weak(
                ui,
                format!(
                    "Following: fifths {:+}, thirds {:+} beyond the offsets",
                    view.followed.threes, view.followed.fives
                ),
            );
        }
        // Shape editing sits beside the selector, outside every fold: it is the
        // control a passage is composed with, and it edits the map shown above.
        let mut editing = view.edit_shape;
        if crate::widgets::checkbox(ui, &mut editing, "Edit shape on lattice")
            .on_hover_text(
                "Click a lattice node to move its note there. Edits save to the selected \
                 map, so its Map automation plays the new shape from the next attack.",
            )
            .changed()
        {
            params.edit_lattice_map(MapEdit::EditShape(editing));
        }
        crate::widgets::button_row(ui, |ui| {
            if ui.add_enabled(view.can_undo, egui::Button::new("Undo shape edit")).clicked() {
                params.edit_lattice_map(MapEdit::Undo);
            }
            let duplicate = egui::Button::new("Duplicate as new map");
            if ui
                .add_enabled(!view.full && view.playback.map.is_some(), duplicate)
                .on_hover_text("Copy this shape to a new saved map and select it.")
                .clicked()
            {
                params.edit_lattice_map(MapEdit::Duplicate);
            }
        });
        if view.full {
            crate::widgets::label(ui, egui::RichText::new("All 128 stable map identities have been used.").color(theme::armed()));
        }
        subsection(ui, "Map offsets", |ui| {
        let mut fine = view.offsets.fine;
        let mut extension = view.offsets.extension;
        egui::Grid::new("map-offsets").show(ui, |ui| {
            for heading in ["Axis", "Fine", "Coarse", "Total"] {
                crate::widgets::weak(ui, heading);
            }
            ui.end_row();
            for (label, fine, extension, axis) in [
                ("Fifths", &mut fine.threes, &mut extension.threes, MapAxis::Fifths),
                ("Thirds", &mut fine.fives, &mut extension.fives, MapAxis::Thirds),
                ("Harmonic sevenths", &mut fine.sevens, &mut extension.sevens, MapAxis::Sevenths),
            ] {
                crate::widgets::label(ui, label);
                for (lane, value, scale) in [
                    (MapOffsetLane::Fine, &mut *fine, 1),
                    (MapOffsetLane::Extension, &mut *extension, EXTENSION_STEP),
                ] {
                    let response = ui.add(
                        egui::DragValue::new(value)
                            .range(-OFFSET_LIMIT..=OFFSET_LIMIT)
                            .speed(0.1)
                            .custom_formatter(move |value, _| {
                                format!("{:.0}", value * f64::from(scale))
                            })
                            .custom_parser(move |text| {
                                let value: i32 = text.trim().parse().ok()?;
                                (value % scale == 0).then_some(f64::from(value / scale))
                            }),
                    );
                    let one_shot =
                        response.changed() && !response.dragged() && !response.drag_stopped();
                    if response.drag_started() || one_shot {
                        params.edit_lattice_map(MapEdit::BeginOffset(axis, lane));
                    }
                    if response.changed() {
                        params.edit_lattice_map(MapEdit::Offset(axis, lane, *value));
                    }
                    if response.drag_stopped() || one_shot {
                        params.edit_lattice_map(MapEdit::EndOffset(axis, lane));
                    }
                }
                crate::widgets::label(ui, (*fine + EXTENSION_STEP * *extension).to_string());
                ui.end_row();
            }
        });
        })
        .on_hover_text(
            "Maps save shape only; these move it. Automate Fine for single steps and Coarse \
             for steps of 10. The two lanes add together.",
        );
        subsection(ui, "Manage selected saved map", |ui| {
            let key = ui.id().with(("rename-map", selected));
            let mut renamed = ui.data(|data| data.get_temp::<String>(key)).unwrap_or_else(|| name.into());
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut renamed).desired_width(140.0));
                if ui.button("Rename").clicked() { params.edit_lattice_map(MapEdit::Rename(selected, renamed.clone())); }
            });
            ui.data_mut(|data| data.insert_temp(key, renamed));
            crate::widgets::button_row(ui, |ui| {
                if ui.button("Move earlier in list").clicked() { params.edit_lattice_map(MapEdit::MoveEarlier(selected)); }
                if ui.button("Delete saved map").on_hover_text("Automation for this identity will pass through without correction. The identity is never reused.").clicked() { params.edit_lattice_map(MapEdit::Delete(selected)); }
            });
        });
        subsection(ui, "Assignments and sounding intervals", |ui| {
            if let Some(map) = view.playback.map {
                for (midi, name) in MIDI_LABELS.iter().enumerate() {
                    let p = map.node(midi as i64);
                    ui.monospace(format!(
                        "{name:2} · {:+.2}¢ · ({}, {}, {})",
                        map.correction(midi as i64, state.runtime.tuning) as f64 / 1e6,
                        p.threes,
                        p.fives,
                        p.sevens
                    ));
                }
            }
            let mut voices: Vec<_> = state
                .runtime
                .tracker
                .voices()
                .filter(|v| v.state == harmonigraph_core::VoiceState::Held)
                .collect();
            voices.sort_by(|a, b| a.pitch.total_cmp(&b.pitch));
            if let Some(lowest) = voices.first() {
                for voice in &voices[1..] {
                    crate::widgets::label(ui, format!(
                        "{}–{} · {:.2}¢",
                        MIDI_LABELS[lowest.note as usize % 12],
                        MIDI_LABELS[voice.note as usize % 12],
                        (voice.pitch - lowest.pitch) * 100.0
                    ));
                }
            }
        });
        mode
    })
    // A folded section changes nothing, so the engine stands as it was.
    .unwrap_or(view.playback.engine)
}
