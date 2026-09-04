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
use crate::{theme, SharedState};
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
    state: &mut SharedState,
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
        *state.view.temper_mut(comma) = false;
        params.set(key, value);
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
fn live_tuning(state: &SharedState, params: &dyn ParamBackend) -> harmonigraph_core::Tuning {
    let mut tuning = crate::params::tuning_from_params(params);
    for comma in tuning::Comma::ALL {
        if state.view.tempers(comma) {
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
fn comma_controls(ui: &mut egui::Ui, state: &mut SharedState) {
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
                let auto_on = state.view.temper_auto(comma);
                crate::widgets::toggle_switch(
                    ui,
                    state.view.temper_mut(comma),
                    comma.temperament(),
                )
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
                // Auto-detect. Switching it ON re-opens the question on the tuning
                // already loaded — without clearing the verdict it would engage
                // nothing until the tuning next moved, since `begin_frame` records
                // every tuning it sees whether the detect is running or not.
                // Switching it off leaves the mode where it is, with the switch
                // beside it still live.
                let auto = crate::widgets::toggle_switch(ui, state.view.temper_auto_mut(comma), "")
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
                ui.end_row();
            }
        });
    });
}

pub(super) fn tuning_pane(
    ui: &mut egui::Ui,
    state: &mut SharedState,
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
        match comma_deriving(key, &state.view) {
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
        if ui
            .button("12-TET")
            .on_hover_text(
                "Equal-tempered steps — 700, 400, 1000 cents. Matches a plain \
                 MIDI keyboard.",
            )
            .clicked()
        {
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
    section(ui, "Temperaments");
    comma_controls(ui, state);

    // Hovering a lattice node deliberately reports NOTHING here. Growing a
    // "Hovered: (t, f, s) = pitch" line whenever the pointer is over a node
    // makes the controls below it jump down and back as the pointer crosses
    // the lattice — a readout in one pane moving another pane's buttons.
    // `state.hovered` drives the lattice's own highlight, which is where a
    // hover belongs.
}
