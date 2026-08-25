//! The controls that are not bars: the two switches a mode is engaged from, and
//! the three helpers a settings pane lays a row of buttons out with.

use egui::{CornerRadius, Response, Sense, TextEdit, TextStyle, Ui, Vec2};

use crate::theme;

/// Track size of a [`toggle_switch`] pill.
const SWITCH_SIZE: Vec2 = Vec2::new(26.0, 15.0);

/// A labeled sliding-knob switch for boolean *modes* (Meantone, Learn).
/// Buttons with a `selected` fill read exactly like the momentary preset
/// buttons they sit next to (Just, 12-TET); the pill-and-knob shape is
/// unmistakably persistent state.
///
/// Toggle vs checkbox, the house rule: a switch means "this mode is
/// ENGAGED" — an ongoing behavior with side effects (Learn keeps
/// rewriting tuning params; Meantone locks the third), especially next
/// to action buttons it could be confused with. A checkbox means
/// "include this element" — a display preference among peers in a
/// settings stack (Fill, Peak hold, Note labels, ...). When adding a
/// boolean control, default to a checkbox unless it's a mode that keeps
/// acting after the click.
pub fn toggle_switch(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        TextStyle::Button.resolve(ui.style()),
        theme::text(),
    );
    let scale = theme::ui_scale(ui.ctx());
    let switch = SWITCH_SIZE * scale;
    let gap = 6.0 * scale;
    // A row's height, though the pill itself is shorter than one. Asked for
    // here rather than inherited, because `allocate_exact_size` means exactly:
    // the `interact_size` floor that brings egui's own controls up to the row
    // never applies to a widget that names its own size, so a switch that asked
    // only for its pill would be the short row in every pane it appears in —
    // and in the Commas table, which is a `Grid` taking each row's height from
    // the cells in it, a short row all the way down.
    let desired = Vec2::new(
        switch.x + gap + galley.size().x,
        theme::row_height(scale).max(switch.y).max(galley.size().y),
    );
    let (rect, mut response) = ui.allocate_exact_size(desired, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, label)
    });

    if ui.is_rect_visible(rect) {
        // Same animation the stock egui toggle demo uses: the knob glides,
        // the track cross-fades.
        let t = ui.ctx().animate_bool_responsive(response.id, *on);
        let track = egui::Rect::from_min_size(
            egui::pos2(rect.left(), rect.center().y - switch.y / 2.0),
            switch,
        );
        let radius = track.height() / 2.0;
        let mix = |a: egui::Color32, b: egui::Color32| -> egui::Color32 {
            egui::lerp(egui::Rgba::from(a)..=egui::Rgba::from(b), t).into()
        };
        let painter = ui.painter();
        painter.rect_filled(track, radius, mix(theme::well(), theme::accent_active()));
        if response.hovered() || response.dragged() {
            painter.rect_stroke(
                track,
                radius,
                egui::Stroke::new(1.0, theme::accent_edge()),
                egui::StrokeKind::Inside,
            );
        }
        let knob_x = egui::lerp((track.left() + radius)..=(track.right() - radius), t);
        painter.circle_filled(
            egui::pos2(knob_x, track.center().y),
            radius - 2.5 * scale,
            theme::text(),
        );
        painter.galley(
            egui::pos2(track.right() + gap, rect.center().y - galley.size().y / 2.0),
            galley,
            theme::text(),
        );
    }
    response
}

/// A record control that doubles as its own live indicator. Press to arm; the
/// dot shows the state — a hollow ring when idle, a breathing dot while armed
/// and waiting for the transport, a solid dot while actually capturing. Press
/// again to stop (which the On-disarm trigger needs, and which serves as a
/// manual early stop under the others).
///
/// Recording is a mode with ongoing side effects, the reason a plain switch was
/// chosen before — but its "off" is usually reached automatically (the
/// transport stopping, a loop ending), so a two-way slider overstated the
/// manual control. A record button that pulses while writing says "capturing a
/// file" at least as clearly, without pretending you drag it both ways.
pub fn record_button(ui: &mut Ui, on: &mut bool, rolling: bool, label: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        TextStyle::Button.resolve(ui.style()),
        theme::text(),
    );
    let scale = theme::ui_scale(ui.ctx());
    let dot_r = 5.0 * scale;
    let gap = 8.0 * scale;
    let pad_x = 10.0 * scale;
    let inner = Vec2::new(dot_r * 2.0 + gap + galley.size().x, galley.size().y.max(dot_r * 2.0));
    // A row's height, asked for the same way and for the same reason as
    // [`toggle_switch`]'s: naming an exact size opts out of the floor that
    // brings every other button to the row. A padding of its own would make
    // this the one control in the Video pane standing taller than the bars
    // under it, which reads as the pane being out of alignment rather than as
    // the button being important — and what makes it important is its panel,
    // its dot and its pulse, none of which cost height.
    let height = theme::row_height(scale).max(inner.y);
    let (rect, mut response) =
        ui.allocate_exact_size(Vec2::new(inner.x + 2.0 * pad_x, height), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), *on, label)
    });

    if ui.is_rect_visible(rect) {
        // Breathing while armed-and-waiting; solid once the transport rolls.
        // Keep repainting so the breath animates even when nothing else moves.
        let alpha = if *on && !rolling {
            let t = ui.ctx().input(|i| i.time);
            ui.ctx().request_repaint();
            0.4 + 0.6 * (0.5 + 0.5 * (t * std::f64::consts::TAU * 1.1).sin()) as f32
        } else {
            1.0
        };
        let painter = ui.painter();
        let bg = if response.hovered() { theme::panel() } else { theme::well() };
        painter.rect_filled(rect, CornerRadius::same(theme::control_radius(scale)), bg);
        if response.hovered() {
            painter.rect_stroke(
                rect,
                CornerRadius::same(theme::control_radius(scale)),
                egui::Stroke::new(1.0, theme::accent_edge()),
                egui::StrokeKind::Inside,
            );
        }
        let dot = egui::pos2(rect.left() + pad_x + dot_r, rect.center().y);
        if *on {
            painter.circle_filled(dot, dot_r, theme::armed().gamma_multiply(alpha));
        } else {
            painter.circle_stroke(
                dot,
                dot_r - 0.75 * scale,
                egui::Stroke::new(1.5, theme::text_dim()),
            );
        }
        painter.galley(
            egui::pos2(
                rect.left() + pad_x + dot_r * 2.0 + gap,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            theme::text(),
        );
    }
    response
}

/// A single-line text field a row high, for the one settings row that holds one
/// (the camera preset's name).
///
/// A text field is the only control here that egui sizes from neither
/// `interact_size` nor a size handed to it: a `TextEdit` is its font's row plus
/// its own margin and nothing else, so the `interact_size` floor that brings
/// every other control to [`theme::ROW_HEIGHT`] does not reach it, and neither
/// does `add_sized`. Its default margin is a whole-point constant besides —
/// 2 points top and bottom at every [chrome scale](theme::ui_scale) — which is
/// a tenth of a row at the design size and a seventh of one at 0.7. Left alone
/// it is the one row in the dock that is not a row high.
///
/// The margin is therefore the lever, and it takes the largest whole number of
/// points that still FITS rather than the one nearest the row. egui stores a
/// margin as whole points, so a field can only land on its text plus an even
/// number and 20 is not one of them; rounding up puts the field back over the
/// row and takes the row up with it, which is the whole defect. Rounding down
/// leaves it a point inside a row that the button beside it holds open, where
/// it reads as an inset field rather than as a row out of line.
/// Takes the `Ui` it will be added to, because the margin is measured against
/// the type that `Ui` is carrying; everything else about the field — its hint,
/// its width — is the caller's, so this hands back the builder rather than
/// adding it.
pub fn row_field<'t>(ui: &Ui, text: &'t mut String) -> TextEdit<'t> {
    let scale = theme::ui_scale(ui.ctx());
    let room = theme::row_height(scale) - ui.text_style_height(&TextStyle::Body);
    TextEdit::singleline(text).margin(egui::Margin::symmetric(
        // The side margin is egui's own, scaled: a field's WIDTH is nobody's
        // alignment problem, unlike its height.
        i8::try_from(theme::scaled_points(4, scale)).unwrap_or(i8::MAX),
        (room * 0.5).floor().max(0.0) as i8,
    ))
}

/// A horizontal row of controls in a settings column, wrapping onto further
/// lines when the column is too narrow to hold it.
///
/// Height is not its business: a row starts at `interact_size.y` and grows
/// under the first widget taller than that, and nothing in a settings pane is
/// taller than that — the theme's `interact_size` is [`theme::ROW_HEIGHT`] and
/// every control here is sized by it or, where egui's floor does not reach
/// ([`row_field`], [`toggle_switch`], [`record_button`]), asks for it. So the
/// row is a row high because the things in it are, and a bare label centers on
/// the button beside it without help. A control that overshot would take the
/// row with it and leave everything shorter in it sitting above the line.
///
/// The single row helper, deliberately: a settings pane is a column whose width
/// the dock hands it, and a row that cannot wrap runs its last buttons out past
/// the pane edge where they can be neither read nor clicked. A non-wrapping
/// variant is only ever the wrong choice here, and having one to reach for is
/// what left the panes disagreeing about whether their buttons wrap at all —
/// Projection and Tilt overran a narrow column while Style and Palette wrapped.
///
/// Wrapping settles the harder half too, and not obviously: `horizontal_wrapped`
/// sets the row's wrap mode, so each BUTTON's own label wraps onto a second line
/// rather than extending past its frame. A single button too wide for the column
/// (Orthographic, at any column narrow enough) has nowhere to wrap TO, and would
/// otherwise overrun the pane whatever the row did — and take every control
/// under it along, since egui's `Region::expand_to_include_rect` unions
/// `max_rect` as well as `min_rect`.
pub fn button_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal_wrapped(add).inner
}

/// A selectable option's label, set in MONOSPACE when the label is a bare
/// number ("1080", "16:9", "-1.5") and left alone when it is a word.
///
/// Numbers are monospace everywhere else in this UI — every bar readout, the
/// perf HUD, the lattice text — for the reason digits always want it: one
/// width per glyph, so a column of them lines up and none of them wiggles as
/// it changes. A row of number buttons is the same picture sideways. Set in
/// the proportional face, "1080" and "1440" come out different widths and the
/// row reads as four unrelated words rather than as a scale.
///
/// The FAMILY only, not [`TextStyle::Monospace`], which would also drop the
/// label to the monospace size and leave a number sitting smaller than the
/// words beside it.
///
/// Decided per label rather than per row, because rows mix: the frame-rate row
/// is "Uncapped" beside four numbers, and only the numbers want this.
pub fn option_label(label: &str) -> egui::RichText {
    let text = egui::RichText::new(label);
    // A digit, and nothing but digits and the punctuation numbers wear.
    let numeric = label.chars().any(|c| c.is_ascii_digit())
        && label.chars().all(|c| c.is_ascii_digit() || "+-±.,:/× ".contains(c));
    if numeric {
        text.family(egui::FontFamily::Monospace)
    } else {
        text
    }
}

/// A labelled row of mutually-exclusive choices for `value`: the standard
/// shape of every enum setting in the settings panes.
///
/// Each option is `(value, label, hover hint)`; an empty hint means no
/// tooltip. Adding a variant to a style enum is then one line here rather
/// than another copy of the label/loop/`selectable_value` scaffolding.
///
/// Number labels come out monospace — see [`option_label`], which the rows
/// built by hand out of `selectable_value` call for themselves.
///
/// A row is live or grayed as a WHOLE, and from an `add_enabled_ui` at the
/// call site rather than anything in here: an option that would do nothing is
/// a property of the section's state, not of the option, and a row whose
/// options disagree about it has no honest label to put on the row. That the
/// gate is outside is also what keeps the row wrapping — the scope
/// `add_enabled_ui` opens is a nested layout, and a nested layout inside
/// `button_row`'s `horizontal_wrapped` does not wrap, so a gate reached for
/// per option in here would run the row off the pane and take the section's
/// separators past the edge with it
/// (`no_settings_pane_overruns_a_narrow_column`).
///
/// The body is `Ui::selectable_value`'s: a `Button::selectable` and a click
/// test. The hint shows in both states (egui splits the two), since a grayed
/// option's tooltip is exactly where "and here is what would switch it on"
/// belongs.
pub fn choice_row<T: Copy + PartialEq>(
    ui: &mut Ui,
    name: &str,
    value: &mut T,
    options: &[(T, &str, &str)],
) {
    button_row(ui, |ui| {
        ui.label(name);
        for &(choice, label, hint) in options {
            let mut response =
                ui.add(egui::Button::selectable(*value == choice, option_label(label)));
            if response.clicked() && *value != choice {
                *value = choice;
                response.mark_changed();
            }
            if !hint.is_empty() {
                response = response.on_hover_text(hint);
                response.on_disabled_hover_text(hint);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::probe::{painted, painted_in};

    /// Every text run a `choice_row` of these options paints, as
    /// `text -> (family, size)`.
    fn choice_row_fonts(options: &[(u32, &str, &str)]) -> Vec<(String, egui::FontId)> {
        let mut value = 0u32;
        painted(400.0, |ui| choice_row(ui, "Row", &mut value, options))
            .iter()
            .filter_map(|cs| match &cs.shape {
                egui::Shape::Text(t) => Some((
                    t.galley.text().to_owned(),
                    t.galley.job.sections[0].format.font_id.clone(),
                )),
                _ => None,
            })
            .collect()
    }

    /// An option label that is a bare number is set in the monospace face, and
    /// one that is a word is not — decided per label, since a row can hold
    /// both. The size is the row's own either way: taking the whole monospace
    /// TEXT STYLE would shrink the numbers, leaving "30" visibly smaller than
    /// the "Uncapped" beside it.
    #[test]
    fn number_option_labels_are_monospace_at_the_rows_own_size() {
        let painted = choice_row_fonts(&[
            (0, "Uncapped", ""),
            (1, "30", ""),
            (2, "144", ""),
            (3, "16:9", ""),
            (4, "-1.5", ""),
            (5, "12-TET", ""),
        ]);
        let numbers = ["30", "144", "16:9", "-1.5"];
        let words = ["Row", "Uncapped", "12-TET"];
        let row_size = painted
            .iter()
            .find(|(text, _)| text == "Uncapped")
            .map(|(_, font)| font.size)
            .expect("the row painted no 'Uncapped'");
        for (text, font) in &painted {
            let wanted = if numbers.contains(&text.as_str()) {
                egui::FontFamily::Monospace
            } else {
                assert!(words.contains(&text.as_str()), "unexpected run {text:?}");
                egui::FontFamily::Proportional
            };
            assert_eq!(font.family, wanted, "{text:?} was painted in {:?}", font.family);
            assert_eq!(font.size, row_size, "{text:?} was painted at {}pt", font.size);
        }
        assert_eq!(painted.len(), numbers.len() + words.len(), "a label went unpainted");
    }

    /// A row-builder: lays out a control row, calling back to add its label
    /// and button. Aliased so clippy doesn't flag the nested `dyn FnMut`.
    type RowFn = fn(&mut Ui, &mut dyn FnMut(&mut Ui));

    /// Label center minus button center in a row built by `row`, under the real
    /// theme — which is the whole question here, the theme being what sizes both
    /// of them. Not `__run_test_ui`: that empties the fonts, text measures
    /// zero-height, and the misalignment this guards against never happens.
    fn row_offset(row: RowFn) -> f32 {
        let mut offset = 0.0;
        let ctx = crate::tests::probe::themed();
        let _ = ctx.run_ui(Default::default(), |ui| {
            row(ui, &mut |ui| {
                let label = ui.label("Node style").rect.center().y;
                let button = ui.button("Steady").rect.center().y;
                offset = label - button;
            });
        });
        offset
    }

    /// A bare label sits on the same line as the button beside it, in a
    /// `button_row` and in a plain `horizontal` alike.
    ///
    /// Both, deliberately, because a row is centered by the two being the same
    /// HEIGHT rather than by anything a row helper does — `theme::ROW_HEIGHT`
    /// through `interact_size`, and `every_settings_row_is_one_row_high` is
    /// what holds it. A `button_row` that centered a label its container did
    /// not would mean the height had gone out from under one of them and the
    /// wrapping helper was papering over it.
    #[test]
    fn a_label_sits_on_the_line_of_the_button_beside_it() {
        let plain = row_offset(|ui, add| {
            ui.horizontal(|ui| add(ui));
        });
        assert!(plain.abs() < 0.5, "a plain row's label is off by {plain}px");

        let wrapped = row_offset(|ui, add| {
            button_row(ui, |ui| add(ui));
        });
        assert!(wrapped.abs() < 0.5, "button_row's label is off by {wrapped}px");
    }

    /// A row of buttons too wide for its column stays inside the column: the
    /// buttons take further lines, and a button whose own label cannot fit on
    /// one line wraps that label rather than extending past its frame.
    ///
    /// Both halves come from `horizontal_wrapped` and neither is visible at the
    /// call site, which is the reason to pin them: what the panes need from
    /// [`button_row`] is that nothing it holds can leave the column, and a
    /// non-wrapping row helper looks identical in the code that calls it.
    ///
    /// 90pt because the second half does not start until 95: above that every
    /// label fits on one line, and turning per-button wrapping off changes
    /// nothing the asserts can see. At 90 the widest label wraps to two rows,
    /// leaving 2.2pt of slack on the passing side and failing by 5.2pt without
    /// it. Wider would pin only the first half, which is what 120 did.
    #[test]
    fn a_row_too_wide_for_its_column_wraps_inside_it() {
        const COLUMN: f32 = 90.0;
        let mut rects = Vec::new();
        let _ = painted_in(egui::vec2(COLUMN, 400.0), |ui| {
            button_row(ui, |ui| {
                ui.label("Projection");
                for label in ["Perspective", "Orthographic", "Cabinet"] {
                    rects.push(ui.button(label).rect);
                }
            });
        });
        for (label, rect) in ["Perspective", "Orthographic", "Cabinet"].iter().zip(&rects) {
            assert!(
                rect.right() <= COLUMN + 1.0,
                "{label} reached {} in a {COLUMN}px column",
                rect.right()
            );
        }
        // And they really did stack rather than all landing on one line.
        assert!(
            rects[2].top() > rects[0].top(),
            "three wide buttons stayed on one line: {rects:?}"
        );
        // The second half, made self-evident rather than incidental: a button
        // taller than a row is one whose label took a second line, a row being
        // exactly what a one-line button stands at (`every_settings_row_is_one_row_high`).
        assert!(
            rects.iter().any(|r| r.height() > theme::ROW_HEIGHT + 5.0),
            "no label wrapped, so only the row-wrap half is under test: {rects:?}"
        );
    }
}
