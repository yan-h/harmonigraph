//! The Display tab's Colors page: both color tables, and the light on them.
//!
//! Note colors are the one table every pitch-colored shape reads — the
//! lattice's discs and octave glyphs, the trail, and the Analyzer's note
//! ribbons — written by the gradient and the Color range, with Bloom the light
//! riding on the result, post-process on both pictures at once. Heatmap colors
//! are the other table: level->color, read by the spectrogram and by the Spiral
//! that draws the same frame.
//!
//! One page for the two, because color is what a reader comes here holding
//! rather than a property of either picture: the question is "what color is
//! this", and answering it in the pane that draws the picture files one gradient
//! under the lattice and the other under the analyzer with nothing saying they
//! are the same kind of thing. Everything else about a played note's own layers
//! is [`super::nodes`], and the text on them [`super::labels`].

use super::section;
use crate::params::{ParamBackend, ParamKey};
use crate::widgets::{button_row, GradientPreview, SpectrumBar, SpreadBar, ValueBar};
use crate::SharedState;
use harmonigraph_scene::ViewConfig;

/// The two color tables in the order they are reached for: the notes' own —
/// gradient, the pitch span it is spread over, the halo the result is bloomed
/// with — and then the heatmap's.
///
/// A plain heading rather than `section`: this is the top of the page body, and
/// the leading rule `section` draws would sit directly under the page picker.
pub(super) fn color_pane(ui: &mut egui::Ui, state: &mut SharedState, params: &dyn ParamBackend) {
    ui.heading("Note colors");
    ui.weak(
        "Colors every note in every pane — the lattice, the trail, the ribbons, \
         the audio ring.",
    );
    // The gradient above the range because it is the coarser of the two: it
    // says what the colors ARE, the range says which pitches they are spread
    // over. Both feed the one table every pitch-colored shape reads, so a
    // change here repaints the discs, the octave glyphs, the trail and the
    // piano roll together.
    spectrum_group(ui, &mut state.view);
    super::param_range_bar(
        ui,
        params,
        (ParamKey::DarkestPitch, ParamKey::BrightestPitch),
        0.0..=120.0,
        crate::COLOR_RANGE_MIN_SPAN,
        "Color range",
        super::pitch_readout,
    )
    .on_hover_text(
        "Which pitches the gradient spans: the low end takes its first color, \
         the high end its last. Colors only — it moves no picture. Drag either \
         end, or between them to slide the range.",
    );
    bloom_bar(ui, &mut state.view);
    section(ui, "Heatmap colors");
    ui.weak("The spectrogram's and the Spiral's level→color table.");
    spectrogram_gradient_group(ui, &mut state.spectrum_config);
}

/// Bloom: the halo the bright end of the gradient blows out into.
///
/// Here rather than with a node layer, which is what its name would suggest.
/// It feeds on the brightness the bars above set, and it is a post-process on
/// BOTH pictures off this one number — the lattice's chain and the piano
/// roll's own (`spectral::roll`) — so it is dialled with the colors rather
/// than with the core, the glyphs or the marks, none of which it belongs to.
fn bloom_bar(ui: &mut egui::Ui, view: &mut ViewConfig) {
    // 0 = off (the renderer skips the whole post-process chain), so the bar
    // doubles as the toggle.
    ValueBar::new(&mut view.bloom_strength, 0.0..=1.5, "Bloom")
        .show(ui)
        .on_hover_text(
            "Soft halo around bright notes, on the lattice and the Analyzer's \
             ribbons alike. 0 turns it off.",
        );
}

/// The pitch gradient as a picture over three bars: the gradient itself across
/// the top, and under it the arc on the spectrum bar, the brightness pair on
/// one of its own, and the chroma pair on another. Each bar is a picture of
/// what its numbers COMPOSE rather than a row per number, which is what keeps a
/// six-number gradient down to three rows and a preview.
///
/// **The preview stands above all three because it answers all three.** It is
/// the only thing here that shows what the six knobs make together — each bar
/// below can only draw its own two — so it belongs to the group rather than to
/// any one bar, and a reader dialling any of them watches the same picture.
/// The order is the reading order: the result first, then the three settings
/// that write it, coarsest first.
///
/// One column of full-width bars, like every other settings group — which is
/// the reason the spectrum is a bar rather than the hue WHEEL a circular value
/// naturally asks for. A wheel large enough to grab is 148pt, six bars of
/// height, and the way to recover that height is to set it BESIDE the bars
/// below; that breaks the rule a test pins instead — a bar in a settings pane
/// is the width of its column, so that dragging the column narrower narrows
/// all of them together
/// (`every_bar_in_a_settings_pane_is_the_width_of_the_pane`). The spectrum bar
/// is the one exception the test allows, and the SIZE of the exception is the
/// point: it gives up 20pt of a 400pt column to the flip button and narrows
/// with the column for the rest, where knobs beside a wheel would be 284pt of
/// 400 and would not.
///
/// A bar costs one row and says the same thing — see [`SpectrumBar`] for how a
/// circle fits on one, for why the flip and the arc share that row rather than
/// taking two, and for why the bar's own name is the one text run in the dock
/// drawn dark.
fn spectrum_group(ui: &mut egui::Ui, view: &mut ViewConfig) {
    // The row first, the colors last — see [`GradientPreview`]: read where it
    // stands, the picture would spend every frame of every drag below it one
    // frame behind the bar being dragged.
    let preview = GradientPreview::reserve(ui);
    SpectrumBar::new(&mut view.pitch_gradient).show(ui).on_hover_text(
        "How much of the color circle the gradient uses, and where it starts. \
         Drag the handle to widen or narrow the arc, the track to rotate it; \
         double-click resets. The end button reverses direction.",
    );
    SpreadBar::brightness(&mut view.pitch_gradient).show(ui).on_hover_text(
        "How bright each end of the pitch range draws: the first number is the \
         lowest note, the second the highest. Drag one end past the other to \
         flip which end is bright; double-click resets.",
    );
    SpreadBar::chroma(&mut view.pitch_gradient).show(ui).on_hover_text(
        "How vivid each end of the pitch range draws — 100% is as vivid as the \
         screen allows, 0 is grey. The first number is the lowest note. \
         Double-click resets.",
    );
    preview.show(ui, &view.pitch_gradient).on_hover_text(
        "The result: the gradient every note is colored by, low note on the \
         left. A picture, not a control — the bars above move it.",
    );
}

/// The heatmap's level->color gradient on the same preview and three bars
/// [`spectrum_group`] above dials the lattice's pitch gradient with, over a row
/// of presets.
///
/// Three bars and not six: the group is the gradient itself across the top, and
/// under it the arc on the spectrum bar, the brightness pair on one of its own
/// and the chroma pair on another, each a picture of what its numbers COMPOSE —
/// see [`spectrum_group`], which is the same set over the same type and says
/// why a six-number gradient costs three rows and a preview rather than six
/// rows.
///
/// **What differs is the axis, and only the readouts show it.** There the range
/// is pitch, so a bar's two ends are the lowest and highest notes; here it is
/// the analyzer's Level, so they are silence and a full bucket. The bars
/// themselves cannot tell — they are handed a [`harmonigraph_scene::Gradient`]
/// and a home to reset to, and nothing in either names an axis — so what says
/// which is the tooltip, and the tooltips below are written for level rather
/// than shared with the group above.
///
/// That difference is why the two are separate groups under separate headings
/// on one page rather than one group: they are two tables read by different
/// code for different quantities, and a reader dialling either wants the axis
/// its bars are named for.
///
/// **The presets come first**, ahead of the preview the group above opens with,
/// and deliberate: a heatmap palette is a thing people pick by name before they
/// dial it, and the four names are the whole of what a heatmap offers before it
/// offers any knobs at all. They write the bars below and are not a mode — see
/// [`crate::SpectrogramPreset`]. The preview then sits between the names and the
/// bars, which is where both of them are read against it.
fn spectrogram_gradient_group(ui: &mut egui::Ui, cfg: &mut crate::SpectrumConfig) {
    use crate::SpectrogramPreset;

    // The gradient a double-click on any of the three goes home to. The fresh
    // heatmap's, NOT the lattice's, which is what the bars assume when a caller
    // names none: a heatmap resetting onto the pitch gradient's arc would land
    // on a picture the spectrogram has never opened on, and the bars carry no
    // text entry to dial it back with.
    let home = crate::SpectrumConfig::default().spectrogram_gradient;
    button_row(ui, |ui| {
        ui.label("Palette").on_hover_text(
            "Four starting looks, written straight into the bars below. A look \
             is six numbers, not a mode: once a bar moves, the picture is the \
             picture.",
        );
        for preset in SpectrogramPreset::ALL {
            if ui
                .button(preset.label())
                .on_hover_text(preset.hint())
                .clicked()
            {
                cfg.spectrogram_gradient = preset.gradient();
            }
        }
    });
    // The row first, the colors last — see [`GradientPreview`].
    let preview = GradientPreview::reserve(ui);
    SpectrumBar::new(&mut cfg.spectrogram_gradient).home(home).show(ui).on_hover_text(
        "How much of the color circle the heatmap uses, and where it starts. \
         Drag the handle to widen or narrow the arc, the track to rotate it; \
         double-click resets. The end button reverses direction.",
    );
    SpreadBar::brightness(&mut cfg.spectrogram_gradient).home(home).show(ui).on_hover_text(
        "How bright each end of the level range draws: the first number is \
         silence, the second a full bucket. Silence usually wants 0 — the \
         heatmap lies on a black bed. Double-click resets.",
    );
    SpreadBar::chroma(&mut cfg.spectrogram_gradient).home(home).show(ui).on_hover_text(
        "How vivid each end of the level range draws — 0 is grey (Mono), 100% \
         as vivid as the screen allows. The first number is silence. \
         Double-click resets.",
    );
    preview.show(ui, &cfg.spectrogram_gradient).on_hover_text(
        "The result: the heatmap's colors, silence on the left, a full bucket \
         on the right. A picture, not a control — the bars above move it.",
    );
}
