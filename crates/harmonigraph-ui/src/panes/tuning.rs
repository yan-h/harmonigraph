//! The Tuning pane: the three prime-interval bars and the tuning-learn
//! controls, then the commas the lattice tempers out.
//!
//! Two sections in one tab because they are two halves of one question: where
//! the lattice's nodes sit in pitch, the bars answering it by number and the
//! commas by identity. Which of those nodes you are then looking at is
//! [`super::view`], and it is a tab rather than a third section here because a
//! tab called Tuning is not where anyone looks for a camera. Everything else in
//! the settings dock is about how what is there gets drawn.

use super::learn_pulse;
use super::param_bar;
use super::section;
use crate::params::{ParamBackend, ParamKey};
use crate::widgets::{button_row, ValueBar};
use crate::{theme, PictureState};
use harmonigraph_core::configuration::ConfigEdit;
use harmonigraph_core::tuning;

#[cfg(test)]
#[path = "tuning_instances_tests.rs"]
mod instance_tests;

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
/// The axes it derives FROM come from [`live_tuning`], so a derived value
/// stacks on a derived value: with both commas tempered the seventh follows
/// the third that meantone is deriving, which is what makes the pair read as
/// septimal meantone rather than as two locks disagreeing about the third.
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
    state: &mut PictureState,
    params: &dyn ParamBackend,
    comma: tuning::Comma,
) {
    let key = derived_key(comma);
    let tuning = live_tuning(state, params);
    let derived = comma.derived(tuning.three_cents(), tuning.five_cents());
    let mut value = derived;
    let response = ValueBar::new(&mut value, key.range(), key.label())
        .decimals(2)
        .unit(1.0, "¢")
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
        state.runtime.edit_tuning(
            &mut state.appearance,
            params,
            ConfigEdit::unlock(comma, tuning::microcents(value)),
        );
    }
    if response.drag_stopped() {
        params.end_set(key);
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

/// The tuning param a comma derives — where its identity lands, and so which
/// bar goes over to [`tempered_bar`] while it is tempered out.
fn derived_key(comma: tuning::Comma) -> ParamKey {
    match comma {
        tuning::Comma::Syntonic => ParamKey::Five,
        tuning::Comma::SeptimalKleisma => ParamKey::Seven,
    }
}

/// What each tuning bar says, for the plain (untempered) bar. A derived axis
/// draws [`tempered_bar`] instead, whose hover names the lock that is holding
/// it rather than the axis.
fn tuning_hint(key: ParamKey) -> &'static str {
    match key {
        ParamKey::COffset => {
            "Where C sits, in cents from standard. Moves every node's pitch \
             together."
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
             cents. Also decides the Notes pane's node column and the \
             Analyzer's off-lattice band."
        }
        // Not on this pane: Fade is a node setting and the two pitch ends are
        // the Colors page's Color range.
        ParamKey::Fade | ParamKey::DarkestPitch | ParamKey::BrightestPitch => "",
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
fn comma_controls(ui: &mut egui::Ui, state: &mut PictureState, params: &dyn ParamBackend) {
    // A table, because the rows answer the same two questions in the same
    // order and a reader compares DOWN the columns: which temperament, and is
    // it engaging by itself. The comma each one tempers out is in the hover
    // rather than a column of its own — a ratio is what the temperament MEANS
    // rather than something to pick a row by, and every column here has to
    // survive a settings column dragged narrow.
    //
    // The Auto heading is its switches' label, which is what lets them stay
    // bare; a labelled switch in every cell would not fit.
    //
    // A table cannot wrap, and even two columns of it need about 135pt — more
    // than a settings column dragged to its narrowest holds, where every other
    // control here either wraps or elides. So it scrolls sideways inside its
    // own clip rather than widening the pane around it, which is also what
    // keeps the section rule under it at the pane's width
    // (`Region::expand_to_include_rect` unions `max_rect`, so an over-wide
    // child moves everything below it out too). It shrinks to the table at any
    // width that fits one, which is every width the column actually opens at.
    //
    // The bar it scrolls by runs UNDER the table, where the pane has no margin
    // to spare it, so the lane comes out of the area's height — a row of cells
    // with a scroll bar drawn across its feet is the alternative.
    theme::reserve_scroll_gutter(ui);
    egui::ScrollArea::horizontal().show(ui, |ui| {
        egui::Grid::new("commas").num_columns(2).show(ui, |ui| {
            for heading in ["Temper", "Auto"] {
                ui.label(egui::RichText::new(heading).color(theme::text_dim()));
            }
            ui.end_row();

            for comma in tuning::Comma::ALL {
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
                // The hover is where the comma itself is named, so it leads with
                // the ratio: it is the thing a reader came to this section for,
                // and the switch beside it says only which temperament drops it.
                let auto_on = state.appearance.view.temper_auto(comma);
                let mut on = state.appearance.view.tempers(comma);
                let temper = crate::widgets::toggle_switch(ui, &mut on, comma.temperament())
                    .on_hover_text(format!(
                        "{} — the {} ({:.2}¢). {} locks the {} to {}, and note names are \
                         respelled to match{}",
                        comma.ratio(),
                        comma.comma_name(),
                        comma.size_cents(),
                        comma.temperament(),
                        comma.derived_axis_name(),
                        comma.derived_from(),
                        if auto_on {
                            ". Auto engages it too, and switching it off here holds until the \
                             tuning changes"
                        } else {
                            ""
                        },
                    ));
                if temper.changed() {
                    let mut edit = ConfigEdit::default();
                    edit.tempered[comma.index()] = Some(on);
                    state.runtime.edit_tuning(&mut state.appearance, params, edit);
                }
                // Auto-detect. Switching it ON re-opens the question on the tuning
                // already loaded — without clearing the verdict it would engage
                // nothing until the tuning next moved, since `begin_frame` records
                // every tuning it sees whether the detect is running or not.
                // Switching it off leaves the mode where it is, with the switch
                // beside it still live.
                let mut auto_on = state.appearance.view.temper_auto(comma);
                let auto =
                    crate::widgets::toggle_switch(ui, &mut auto_on, "").on_hover_text(format!(
                        "Engage {} by itself whenever the {} lands within {}¢ of {} — from a \
                         preset, a learned chord, or a drag of any bar",
                        comma.temperament(),
                        comma.derived_axis_name(),
                        tuning::TEMPER_TOLERANCE,
                        comma.derived_from(),
                    ));
                if auto.changed() {
                    let mut edit = ConfigEdit::default();
                    edit.auto[comma.index()] = Some(auto_on);
                    state.runtime.edit_tuning(&mut state.appearance, params, edit);
                }
                ui.end_row();
            }
        });
    });
}

pub(super) fn tuning_pane(
    ui: &mut egui::Ui,
    state: &mut PictureState,
    params: &dyn ParamBackend,
    now: f64,
) {
    // A plain heading rather than `section`: this is the top of the pane, and
    // the leading rule `section` draws would be a line under nothing. Matches
    // the Display tab's pages, which open the same way under their picker.
    ui.heading("Tuning");
    ui.weak("Set the pitch of each lattice step. 100 cents (¢) equals one semitone.");
    // Tuning sliders. A comma that is tempered out derives one of these axes
    // (meantone the major third, marvel the harmonic seventh), so that axis's
    // bar shows the derived value and is where the mode is released.
    for &key in &ParamKey::TUNING {
        match comma_deriving(key, &state.appearance.view) {
            Some(comma) => tempered_bar(ui, state, params, comma),
            None => {
                param_bar(ui, params, key).on_hover_text(tuning_hint(key));
            }
        }
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
                    tempered: [Some(false); 2],
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
        // v1's tuning-learn mode: while engaged, the tuning re-learns
        // instantly whenever the set of held notes changes (see root_ui).
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
            // Pulsing armed ring so the engaged mode can't be missed.
            ui.painter().rect_stroke(
                learn.rect.expand(2.0),
                egui::CornerRadius::same(6),
                egui::Stroke::new(2.0, theme::armed().gamma_multiply(learn_pulse(now))),
                egui::StrokeKind::Outside,
            );
        }
    });
    keyboard_controls(ui, state, params);

    // Which commas the lattice tempers out: the same question as the bars
    // above (what IS this tuning), but the answer is a set of identities
    // rather than three numbers, so it gets its own heading.
    section(ui, "Temperaments");
    comma_controls(ui, state, params);
    let configuration_notice = if state.runtime.configuration_status & 3 != 0 {
        ui.colored_label(
            theme::armed(),
            "Learning unavailable: configuration or held state is incomplete. Reset to recover.",
        );
        true
    } else if state.runtime.configuration_status & 4 != 0 {
        ui.weak("Tuning applied; host notification was rejected");
        true
    } else if state.runtime.configuration_status & 8 != 0 {
        ui.weak("Tuning change refused: pending command storage is full");
        true
    } else {
        false
    };

    adaptive_controls(ui, state, params);

    // After every control: `configuration_pending` can come and go between
    // consecutive frames while a drag submits policy edits and the audio
    // thread adopts them. A conditional row above Adaptive tuning moves the
    // bar still held under the pointer, making the gesture feed back into its
    // own value and the whole section alternate between two positions. The
    // persistent fault notices stay prominent above the controls; unlike this
    // ordinary pending transition, they do not alternate within the gesture.
    if !configuration_notice && state.runtime.configuration_pending {
        ui.weak("Tuning change pending audio adoption");
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
    ui.collapsing("Keyboard", |ui| {
        for (value, label) in p.keyboard.iter_mut().zip(["Fifth", "Third", "Seventh"]) {
            *value = adaptive_value(ui, *value as u32, 0..=1_200_000_000, 1_000_000.0, label, "¢")
                as i32;
        }
        if ui.button("Derive from fifth").clicked() {
            p.keyboard = tuning::fifth_generated(p.keyboard[0]);
        }
        let [third, seventh] = tuning::fifth_generated_steps(p.keyboard[0]).map(fifths);
        ui.weak(format!("From the fifth: third is {third}, seventh is {seventh}."));
    })
    .header_response
    .on_hover_text(
        "A key may only become a lattice node this keyboard would play at the pitch the key \
         sent; an attack bent off every key is chosen from every node. Learn sets it from the \
         fifth it hears.",
    );
    if p != before {
        state.runtime.edit_tuning(
            &mut state.appearance,
            params,
            ConfigEdit { policy: Some(p.sanitize()), ..Default::default() },
        );
    }
}

fn fifths(steps: i32) -> String {
    let n = steps.unsigned_abs();
    let plural = if n == 1 { "" } else { "s" };
    format!("{n} fifth{plural} {}", if steps < 0 { "down" } else { "up" })
}

fn adaptive_controls(ui: &mut egui::Ui, state: &mut PictureState, params: &dyn ParamBackend) {
    section(ui, "Adaptive tuning");
    instance_controls(ui, params);
    let mut p = state.runtime.adaptive_policy;
    let before = p;
    p.harmonic =
        adaptive_value(ui, p.harmonic.into(), 0..=20_000, 1000.0, "Harmonic weight", "") as u16;
    ui.checkbox(
        &mut state.runtime.neighbourhood.visible,
        "Show reachable neighbourhood (input C2–C7)",
    );
    p.pitch_scale =
        adaptive_value(ui, p.pitch_scale.into(), 1..=100, 1.0, "Pitch scale", "¢").max(1) as u16;
    p.radius =
        adaptive_value(ui, p.radius.into(), 1..=5, 1.0, "Neighbourhood steps", "").max(1) as u8;
    ui.label("Allowed axes");
    theme::reserve_scroll_gutter(ui);
    egui::ScrollArea::horizontal().id_salt("adaptive-axes-scroll").show(ui, |ui| {
        egui::ComboBox::from_id_salt("adaptive-axes")
            .selected_text(match p.axes {
                1 => "Fifths",
                2 => "Fifths + thirds",
                _ => "Fifths + thirds + sevenths",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut p.axes, 1, "Fifths");
                ui.selectable_value(&mut p.axes, 2, "Fifths + thirds");
                ui.selectable_value(&mut p.axes, 3, "Fifths + thirds + sevenths");
            });
    });
    ui.collapsing("Context", |ui| {
        p.half_life_ms =
            adaptive_value(ui, p.half_life_ms.into(), 0..=20_000, 1000.0, "Half-life", "s") as u16;
        ui.weak("A note struck or released this long before the newest one counts half. Zero means no decay.");
        p.memory = adaptive_value(ui, p.memory.into(), 0..=24, 1.0, "Released pitches", "") as u8;
        for (value, label, max) in [
            (&mut p.released, "Released : held weight", 1000),
            (&mut p.register_floor, "Register floor", 1000),
            (&mut p.register_falloff, "Register falloff", 4000),
        ] {
            *value = adaptive_value(ui, (*value).into(), 0..=max, 1000.0, label, "") as u16;
        }
        p.tolerance = adaptive_value(
            ui,
            p.tolerance,
            0..=20_000_000,
            1_000_000.0,
            "Same-note tolerance",
            "¢",
        );
        p.silence_ms = adaptive_value(ui, p.silence_ms, 0..=120_000, 1000.0, "Silence reset", "s");
        ui.weak("Silence reset: zero means never.");
        ui.checkbox(&mut p.reset_stop, "Reset context on stop");
        ui.checkbox(&mut p.reset_loop, "Reset context on loop / seek");
    });
    ui.weak(
        "New attacks follow the moving context. Sounding notes keep their adaptive correction.",
    );
    if p != before {
        state.runtime.edit_tuning(
            &mut state.appearance,
            params,
            ConfigEdit { policy: Some(p.sanitize()), ..Default::default() },
        );
    }
}

fn instance_controls(ui: &mut egui::Ui, params: &dyn ParamBackend) {
    use crate::params::InstanceEdit;
    let instances = params.tuning_instances();
    if instances.is_empty() {
        return;
    }
    ui.horizontal(|ui| {
        for (label, retune) in [("Retune all", true), ("Show all", false)] {
            let enabled =
                instances.iter().filter(|row| if retune { row.retune } else { row.show }).count();
            let mut all = enabled == instances.len();
            let mixed = enabled != 0 && !all;
            if ui.add(egui::Checkbox::new(&mut all, label).indeterminate(mixed)).clicked() {
                let value = enabled != instances.len();
                for row in &instances {
                    params.edit_tuning_instance(
                        row.id,
                        if retune {
                            InstanceEdit::Retune(value)
                        } else {
                            InstanceEdit::Show(value)
                        },
                    );
                }
            }
        }
    });
    let selection = ui.id().with("tuning-instance-selection");
    let mut selected = ui.data(|data| data.get_temp::<u64>(selection)).unwrap_or(instances[0].id);
    if !instances.iter().any(|row| row.id == selected) {
        selected = instances[0].id;
    }
    let name_width = (ui.available_width() - 120.0).max(45.0);
    egui::Grid::new("tuning-instances").num_columns(3).spacing([8.0, 6.0]).show(ui, |ui| {
        ui.weak("Instance");
        ui.weak("Retune");
        ui.weak("Show");
        ui.end_row();
        for row in &instances {
            ui.push_id(row.id, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(name_width);
                    let name = if row.name.is_empty() {
                        if row.is_hub {
                            "Harmonigraph input".to_owned()
                        } else {
                            format!("Tune {}", row.id)
                        }
                    } else {
                        row.name.clone()
                    };
                    if ui
                        .add(egui::Button::selectable(selected == row.id, name).truncate())
                        .clicked()
                    {
                        selected = row.id;
                    }
                    let status = if row.misses != 0 {
                        format!("{} held · {} missed", row.held, row.misses)
                    } else {
                        format!("{} held · {} out", row.held, row.notes_out)
                    };
                    ui.small(status).on_hover_text(&row.status);
                    if row.status != "No faults" {
                        ui.colored_label(
                            theme::armed(),
                            egui::RichText::new("Check status").small(),
                        )
                        .on_hover_text(&row.status);
                    }
                });
            });
            let mut retune = row.retune;
            if ui
                .checkbox(&mut retune, "")
                .on_hover_text("Retune notes and contribute to adaptive tuning")
                .changed()
            {
                params.edit_tuning_instance(row.id, InstanceEdit::Retune(retune));
            }
            let mut show = row.show;
            if ui
                .checkbox(&mut show, "")
                .on_hover_text("Show this instance's output notes")
                .changed()
            {
                params.edit_tuning_instance(row.id, InstanceEdit::Show(show));
            }
            ui.end_row();
        }
    });
    ui.data_mut(|data| data.insert_temp(selection, selected));
    if let Some(row) = instances.iter().find(|row| row.id == selected) {
        ui.push_id(row.id, |ui| {
            ui.collapsing("Instance details", |ui| {
                let mut name = row.name.clone();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut name)
                            .hint_text("Instance name")
                            .desired_width(ui.available_width()),
                    )
                    .changed()
                {
                    params.edit_tuning_instance(row.id, InstanceEdit::Name(name));
                }
                ui.label(&row.status);
                ui.small(format!(
                    "{} notes in · {} out · {} missed corrections",
                    row.notes_in, row.notes_out, row.misses
                ));
                if let Some((input, output)) = row.last_pitch {
                    ui.small(format!(
                        "Last attack: {} → {}",
                        monitor_pitch(input),
                        monitor_pitch(output)
                    ))
                    .on_hover_text("Note pitch and per-note expression, before channel pitch bend");
                    ui.small(format!("Correction: {:+.1}¢", output - input));
                }
                ui.label(&row.delay_text);
                if row.max_delay > 1 {
                    let id = ui.id().with("delay-draft");
                    let mut delay = ui.data(|data| data.get_temp::<u32>(id)).unwrap_or(row.delay);
                    ui.horizontal(|ui| {
                        ui.label("Buffers of delay");
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
                    ui.weak("Harmonigraph's own input needs one buffer.");
                }
            });
        });
    }
    if let Some(tuner) = instances.iter().find(|row| !row.is_hub) {
        ui.collapsing("Tuning delay", |ui| {
            let id = ui.id().with("all-delay-draft");
            let mut delay = ui.data(|data| data.get_temp::<u32>(id)).unwrap_or(tuner.delay);
            ui.horizontal(|ui| {
                ui.label("Buffers");
                ui.add(egui::DragValue::new(&mut delay).range(1..=tuner.max_delay));
                if ui.button("Apply to all tuners").clicked() {
                    for row in &instances {
                        if !row.is_hub {
                            params.edit_tuning_instance(row.id, InstanceEdit::Delay(delay));
                        }
                    }
                }
            });
            ui.data_mut(|data| data.insert_temp(id, delay));
            ui.weak(
                "Each tuner reports its own latency. Individual overrides are in Instance details.",
            );
            ui.weak("Harmonigraph's own input stays at one buffer.");
        });
    }
    if ui
        .button("Reset all voices")
        .on_hover_text("Release held notes and clear adaptive tuning context on every instance")
        .clicked()
    {
        params.edit_tuning_instance(instances[0].id, InstanceEdit::Reset);
    }
    ui.weak(
        "Retune off: pass notes through without influencing tuning. Show only affects the picture.",
    );
    ui.separator();
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
) -> u32 {
    let mut value = raw as f32 / scale;
    let mut bar = ValueBar::new(
        &mut value,
        *range.start() as f32 / scale..=*range.end() as f32 / scale,
        label,
    )
    .unit(1.0, unit);
    bar = if scale == 1.0 { bar.integer() } else { bar.decimals(2) };
    if bar.show(ui).changed() {
        (value * scale).round() as u32
    } else {
        raw
    }
}
