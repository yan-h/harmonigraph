//! The chrome scale — what it resizes and what it deliberately does not —
//! and that pointing at a control does not move the row it sits in.

use crate::*;
use super::harness::*;

/// The content width [`settings_pane_at_scale`] gives a pane — the column, not
/// the tab body, which is a margin wider on each side.
const PANE_WIDTH: f32 = 400.0;

/// One settings pane drawn at a given [chrome scale](crate::theme::ui_scale),
/// as the shapes it emitted. The same nesting as
/// [`settings_pane_at_width`] — the dock's clip outside the pane's content box
/// — at a fixed width, because here it is the scale that varies.
fn settings_pane_at_scale(pane: SettingsPane, scale: f32) -> Vec<egui::epaint::ClippedShape> {
    let mut state = fresh();
    state.ui_scale = scale;
    let tab = pane.install(&mut state);
    let ctx = super::probe::themed_scaled(scale);
    tab_body_on(&ctx, &mut state, tab, PANE_WIDTH, PANE_HEIGHT, 0.0).shapes
}

/// Every shape's bottom edge, ignoring the ones that carry no geometry (they
/// answer with an inverted or infinite rect).
fn drawn_bottom(shapes: &[egui::epaint::ClippedShape]) -> f32 {
    shapes
        .iter()
        .map(|cs| cs.shape.visual_bounding_rect())
        .filter(|r| r.is_finite() && r.width() < 1.0e4)
        .fold(0.0f32, |lowest, r| lowest.max(r.bottom()))
}

/// The tallest line of type drawn, which is the half of the scale that reads
/// as "smaller UI" rather than "tighter UI".
fn tallest_text(shapes: &[egui::epaint::ClippedShape]) -> f32 {
    shapes
        .iter()
        .filter_map(|cs| match &cs.shape {
            egui::Shape::Text(t) => Some(t.galley.size().y),
            _ => None,
        })
        .fold(0.0f32, f32::max)
}

/// Turning the scale down spends less of the column on the panel — in type and
/// in the space around it alike, which is what separates this from a font-size
/// control.
#[test]
fn the_ui_scale_shrinks_the_panel_chrome() {
    for pane in [
        SettingsPane::Tab(panes::Tab::Tuning),
        SettingsPane::Tab(panes::Tab::System),
        SettingsPane::Section(Section::Nodes),
    ] {
        let (full, small) =
            (settings_pane_at_scale(pane, 1.0), settings_pane_at_scale(pane, 0.7));

        let (tall, short) = (tallest_text(&full), tallest_text(&small));
        assert!(tall > 0.0, "{pane:?} drew no text to measure");
        assert!(short < tall, "{pane:?} type stayed at {tall} points with the scale at 0.7");

        // The column, not just the glyphs in it: a control's height and the
        // gaps between rows come from the style's spacing, so a pane that only
        // shrank its type would land well short of the type's own ratio.
        let (deep, shallow) = (drawn_bottom(&full), drawn_bottom(&small));
        assert!(
            shallow < deep * 0.85,
            "{pane:?} ran to {shallow} points at 0.7 against {deep} at 1.0 — \
             the spacing is not scaling with the type",
        );
    }
}

/// The scale moves the panel and nothing else: a picture pane handed the same
/// rect draws the same picture whatever the chrome is doing.
///
/// This is what makes the control safe to offer. The lattice, the roll and the
/// spectrogram measure everything off the pane they land in, so a laptop
/// dialling its panel down is not quietly composing a different picture — and
/// the offline renderer, which reaches the same panes through
/// [`draw_pane`](crate::draw_pane) on a context that is never told a scale,
/// cannot disagree with what the plugin showed. The determinism test would not
/// catch it if one did: it renders everything at one scale.
#[test]
fn the_ui_scale_leaves_the_picture_alone() {
    let picture = |scale: f32| {
        let mut state = fresh();
        let backend = RecordingBackend::default();
        // Something to draw: a held voice for the roll and the voice bars, and
        // an analyzed column for the spectrum.
        state.tracker.handle_event(harmonigraph_core::NoteEvent::on(0.5, 0, 60, 1.0));
        let mut bins = [0.0f32; harmonigraph_core::spectrum::SPECTRUM_BINS];
        bins[harmonigraph_core::spectrum::SPECTRUM_BINS / 3] = 0.8;
        state.spectrum.push_history(0.5, &bins);
        // A rect the scale cannot move, so what is being compared is the
        // picture rather than the pane it was given.
        let screen = egui::vec2(600.0, 400.0);
        let out = super::probe::frame_full(&super::probe::themed_scaled(scale), screen, |ui| {
            crate::begin_frame(&mut state, &backend, 1.0);
            crate::draw_pane(ui, crate::Pane::Spectral, &mut state, 1.0);
        });
        // Debug rather than the shapes themselves: they hold floats and
        // texture handles and are not `PartialEq`, and a difference anywhere
        // in one is a difference in the picture.
        out.shapes.iter().map(|cs| format!("{:?}", cs.shape)).collect::<Vec<_>>()
    };

    let (design, small) = (picture(1.0), picture(0.7));
    assert!(!design.is_empty(), "the analyzer drew nothing to compare");
    assert_eq!(design.len(), small.len(), "the chrome scale changed what the analyzer draws");
    for (i, (a, b)) in design.iter().zip(&small).enumerate() {
        assert_eq!(a, b, "shape {i} of the analyzer moved with the chrome scale");
    }
}

/// A tab bar tracks the scale the whole way down, and still leaves room for the
/// arrow that unfolds a pane.
///
/// The bar carries egui_dock's collapse button, and the tempting reading of
/// that button's `TAB_COLLAPSE_BUTTON_SIZE` — 24 points — is a floor the bar
/// must not go under. It is not one: 24 is the button's WIDTH, its rect is as
/// tall as the bar it sits in, and what has to fit vertically is the 10-point
/// arrow centred in it. Flooring the bar at 24 instead left every tab bar in
/// the editor full size below about 0.9, which is what the scale is for.
#[test]
fn a_tab_bar_tracks_the_scale_and_still_fits_the_collapse_arrow() {
    /// egui_dock's `TAB_COLLAPSE_ARROW_SIZE`: the glyph, not the button.
    const ARROW_GLYPH: f32 = 10.0;

    let mut previous = 0.0;
    for scale in [0.7f32, 0.8, 0.9, 1.0, 1.5] {
        let height = crate::theme::tab_bar_height(scale);
        assert!(
            height > previous,
            "a tab bar {height} points tall at scale {scale} did not move from {previous}",
        );
        previous = height;
        assert!(
            height > ARROW_GLYPH,
            "a tab bar {height} points tall at scale {scale} clips the collapse arrow",
        );
    }
    // Proportional, so the bar keeps its share of the chrome rather than
    // levelling off somewhere inside the range.
    let ratio = crate::theme::tab_bar_height(0.7) / crate::theme::tab_bar_height(1.0);
    assert!((ratio - 0.7).abs() < 1.0e-5, "the bar scaled by {ratio} rather than by 0.7");
}

/// The controls a settings row is built from: egui's own `button` (the
/// momentary presets — Just, 12-TET, Reset layout), its `selectable_value`
/// (every [`choice_row`](crate::widgets::choice_row)), its `checkbox` and its
/// `TextEdit` (the camera preset's name), then the two of ours that allocate
/// their own geometry.
#[derive(Clone, Copy, Debug)]
enum Control {
    Button,
    Selectable,
    Checkbox,
    Field,
    Switch,
    Record,
}

/// Every one of them, so a sweep cannot quietly cover four.
const CONTROLS: [Control; 6] = [
    Control::Button,
    Control::Selectable,
    Control::Checkbox,
    Control::Field,
    Control::Switch,
    Control::Record,
];

/// A row of four `kind` controls at `scale`: the rect the ROW came out at,
/// then the rects the four controls allocated inside it, with the pointer
/// parked at `pointer` and optionally held down there.
///
/// The row is reported alongside the controls because it is the thing a reader
/// of a settings pane actually sees the height of. A control is free to be
/// shorter than the row it sits in — the switch's pill is, and a text field is
/// at most scales — and only a control that OVERSHOOTS moves the row.
///
/// Several frames because a widget's visual state comes from the PREVIOUS
/// frame's response: the first frame after the pointer arrives still draws the
/// resting look, and it is the one after it that has to land in the same place.
fn control_row(
    kind: Control,
    scale: f32,
    pointer: Option<egui::Pos2>,
    pressed: bool,
) -> (egui::Rect, Vec<egui::Rect>) {
    let ctx = super::probe::themed_scaled(scale);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 200.0));
    let rects = std::cell::RefCell::new(Vec::new());
    let mut row = egui::Rect::NOTHING;
    // Live state, so a control that reads its own value (the switch's knob, the
    // record dot, the selected choice) is drawn in whatever state the pointer
    // has put it in rather than always in its resting one.
    let mut selected = 1usize;
    let mut flag = false;
    let mut name = String::from("Front");
    for frame in 0..4 {
        rects.borrow_mut().clear();
        let mut events = Vec::new();
        if let Some(pos) = pointer {
            events.push(egui::Event::PointerMoved(pos));
            // Pressed on the first frame only: egui holds the button down
            // until a release event, and a fresh press every frame would read
            // as a click per frame.
            if pressed && frame == 0 {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                });
            }
        }
        let input = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(f64::from(frame)),
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| {
            row = ui.horizontal(|ui| {
                for i in 0..4 {
                    let label = format!("Item {i}");
                    let response = match kind {
                        Control::Button => ui.button(label),
                        Control::Selectable => ui.selectable_value(&mut selected, i, label),
                        Control::Checkbox => ui.checkbox(&mut flag, label),
                        // The Tuning pane's preset-name field, at the width it
                        // asks for there.
                        Control::Field => {
                            let field = crate::widgets::row_field(ui, &mut name)
                                .desired_width(110.0 * scale);
                            ui.add(field)
                        }
                        Control::Switch => crate::widgets::toggle_switch(ui, &mut flag, &label),
                        Control::Record => {
                            crate::widgets::record_button(ui, &mut flag, false, &label)
                        }
                    };
                    rects.borrow_mut().push(response.rect);
                }
            })
            .response
            .rect;
        });
    }
    (row, rects.into_inner())
}

/// Hovering or pressing a control changes how it looks and not where anything
/// is: neither the control's own rect nor the rects of the controls after it in
/// the row.
///
/// egui offers this as `WidgetVisuals::expansion`, a hover swell of a point or
/// two, and it does not hold: the swell is paid back with a negative outer
/// margin on the button's frame, egui stores frame margins as whole points, and
/// the [chrome scale](crate::theme::ui_scale) makes the expansion fractional —
/// so the two round apart and the residue lands in the ALLOCATED size. Measured
/// at scale 1.25 with a 1pt expansion: the hovered button 2pt wider and every
/// button right of it 2pt over; at 1.5, 2pt the other way. In a
/// [`button_row`](crate::widgets::button_row), which wraps, 2pt is also enough
/// to push the last button onto a second line and back as the pointer passes.
///
/// The whole scale range, because which scales drift is exactly the question:
/// at the design size the numbers happen to cancel and nothing moves.
#[test]
fn pointing_at_a_control_leaves_the_row_where_it_is() {
    for kind in CONTROLS {
        for step in 0..=16u8 {
            let scale = 0.7 + 0.05 * f32::from(step);
            let resting = control_row(kind, scale, None, false);
            // The pointer goes to the middle of the FIRST control, so anything
            // that moves has three neighbours to its right to show it in.
            let target = resting.1[0].center();
            for (state, pressed) in [("hovered", false), ("pressed", true)] {
                let pointed = control_row(kind, scale, Some(target), pressed);
                assert_eq!(
                    resting, pointed,
                    "a {state} {kind:?} at scale {scale} moved the row",
                );
            }
        }
    }
}

/// A row of any of the six controls a settings row can be built from stands
/// exactly one [`ROW_HEIGHT`](crate::theme::ROW_HEIGHT), so a pane reads as a
/// column of rows rather than as a stack that changes gauge wherever a button
/// or a text field appears.
///
/// The ROW is what is pinned to the number, and the controls only to not
/// exceeding it, because those are two different questions and only the first
/// is what a reader sees. A control shorter than its row is inset in it — the
/// switch's pill is 15 points in a 20-point row deliberately, and a text field
/// lands a point under at most scales because egui stores its margin as whole
/// points. A control TALLER than its row takes the row with it, which is the
/// misalignment this is here about.
///
/// What holds the row is the `interact_size` FLOOR rather than any of the
/// controls agreeing on a number, and that is the part worth pinning. A button
/// is as tall as its text plus `button_padding`, or the floor, whichever is
/// more, and only the floor is a round height: a frame's margin is whole points
/// too, so a button sized by its padding alone lands on its text plus an even
/// number and can miss a 20-point row either way.
///
/// Which makes the padding what breaks this, quietly and from a distance: raise
/// the type or the padding until their sum clears the floor and the floor stops
/// applying, one control at a time. The sweep is the whole [scale
/// range](crate::theme::UI_SCALE_RANGE) because the sum clears it at some
/// scales before others — the margin rounds to whole points while the type it
/// wraps does not, so the headroom is not the same fraction twice. It is at its
/// narrowest at the two ends: 0.28pt at 0.7, and 0.88pt at 1.5 where the
/// padding rounds up to 2.
///
/// [`every_bar_is_one_row_high`] covers the other half of a settings pane, the
/// bars, which reach the height by allocating it rather than by any floor.
#[test]
fn every_settings_row_is_one_row_high() {
    for kind in CONTROLS {
        for step in 0..=16u8 {
            let scale = 0.7 + 0.05 * f32::from(step);
            let want = crate::theme::row_height(scale);
            let (row, controls) = control_row(kind, scale, None, false);
            assert!(
                (row.height() - want).abs() < 0.01,
                "a row of {kind:?} at scale {scale} stands {}pt high, not {want}pt",
                row.height(),
            );
            for rect in controls {
                assert!(
                    rect.height() <= want + 0.01,
                    "a {kind:?} at scale {scale} stands {}pt high and takes its {want}pt row \
                     up with it",
                    rect.height(),
                );
            }
        }
    }
}

/// Every bar a settings pane draws is one
/// [`ROW_HEIGHT`](crate::theme::ROW_HEIGHT) tall, at every scale.
///
/// The bars are the other half of a settings pane and they reach the height by
/// a different route — each allocates it outright, rather than being grown to
/// it by the `interact_size` floor that catches everything in
/// [`every_settings_row_is_one_row_high`]. Two routes to one number is exactly
/// what drifts, so both are pinned.
///
/// Swept through the real panes rather than by building the six bar types by
/// hand, which is what makes this cover them: `ValueBar`, `RangeBar`,
/// `OctaveStrip`, `SpreadBar` and the render `progress_bar` are all in the
/// panes below, and a seventh added later is covered on the day it is drawn
/// rather than on the day someone remembers to add it to a list here.
///
/// The `SpectrumBar` is a row like the rest, and its width is the one thing
/// that differs — the flip button takes the right end of the row. It is what
/// Color & light is in the sweep for, the pitch gradient's group being the one
/// place the lattice's settings draw one. The gradient preview above it is
/// deliberately SHORTER than a row and is not swept: it paints no well, being
/// a picture rather than a track, so the sniffing below never reaches it.
#[test]
fn every_bar_is_one_row_high() {
    for pane in [
        SettingsPane::Tab(panes::Tab::Tuning),
        SettingsPane::Section(Section::Color),
        SettingsPane::Section(Section::View),
        SettingsPane::Section(Section::Nodes),
        SettingsPane::Section(Section::Analyzer),
        SettingsPane::Tab(panes::Tab::Video),
    ] {
        for step in 0..=16u8 {
            let scale = 0.7 + 0.05 * f32::from(step);
            let want = crate::theme::row_height(scale);
            let shapes = settings_pane_at_scale(pane, scale);
            // Found by WIDTH, never by height: a bar fills the column (the
            // spectrum's track gives its right end to the flip button and is
            // the one exception), so that is a property this test does not
            // depend on. Sniffing for row-high rects instead would drop a
            // mis-sized bar out of the sweep rather than failing on it, which
            // is a test that passes by finding nothing.
            let track = crate::widgets::spectrum_track_width(PANE_WIDTH, scale);
            let mut found = 0;
            for cs in &shapes {
                let egui::Shape::Rect(r) = &cs.shape else { continue };
                let width = r.rect.width();
                if r.fill != crate::theme::well()
                    || !r.rect.is_finite()
                    || ((width - PANE_WIDTH).abs() > 1.0 && (width - track).abs() > 1.0)
                {
                    continue;
                }
                found += 1;
                assert!(
                    (r.rect.height() - want).abs() < 0.01,
                    "{pane:?} at scale {scale} drew a {}pt bar, not {want}pt",
                    r.rect.height(),
                );
            }
            assert!(found > 0, "{pane:?} at scale {scale} drew no bar tracks to measure");
        }
    }
}
