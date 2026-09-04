//! MIDI pitch and audio-level gradients. Each table is shared by every pane
//! that draws its source: MIDI notes use pitch; analyzed audio uses level.
//! Bloom and shadows live on the Lighting page.

use super::section;
use crate::params::{ParamBackend, ParamKey};
use crate::widgets::{button_row, GradientPreview, RangeBar, SpectrumBar, SpreadBar};
use crate::SharedState;
use harmonigraph_scene::ViewConfig;

/// MIDI colors and their pitch range, then audio colors and their level range.
pub(super) fn color_pane(ui: &mut egui::Ui, state: &mut SharedState, params: &dyn ParamBackend) {
    ui.heading("MIDI note colors");
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
        "Pitch color range",
        super::pitch_readout,
    )
    .on_hover_text(
        "Pitches assigned the first and last gradient colors. \
                 Pitches outside this range keep the nearest end color. \
                 Drag an end to resize, or the middle to shift both.",
    );
    section(ui, "Audio level colors");
    spectrogram_gradient_group(ui, &mut state.spectrum_config);
}

fn spectrum_group(ui: &mut egui::Ui, view: &mut ViewConfig) {
    // The row first, the colors last — see [`GradientPreview`]: read where it
    // stands, the picture would spend every frame of every drag below it one
    // frame behind the bar being dragged.
    let preview = GradientPreview::reserve(ui);
    SpectrumBar::new(&mut view.pitch_gradient).show(ui).on_hover_text(
        "Hue range for MIDI notes. \
                 Drag the handle to change its span, the track to rotate it, or the end button to reverse it. \
                 Double-click resets.",
    );
    SpreadBar::brightness(&mut view.pitch_gradient).show(ui).on_hover_text(
        "Brightness at the low and high pitches: 0% is black, 100% is white. \
                 Drag either end; crossing them reverses the brightness ramp. \
                 Double-click resets.",
    );
    SpreadBar::chroma(&mut view.pitch_gradient).show(ui).on_hover_text(
        "Saturation at the low and high pitches: 0% is gray, 100% is the most vivid available color. \
                 Double-click resets.",
    );
    preview
        .show(ui, &view.pitch_gradient)
        .on_hover_text("MIDI note colors from low pitch on the left to high pitch on the right.");
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
            "Starting palettes for audio levels. \
                 Selecting one replaces the color controls below; you can adjust them afterward.",
        );
        for preset in SpectrogramPreset::ALL {
            if ui.button(preset.label()).on_hover_text(preset.hint()).clicked() {
                cfg.spectrogram_gradient = preset.gradient();
            }
        }
    });
    // The row first, the colors last — see [`GradientPreview`].
    let preview = GradientPreview::reserve(ui);
    SpectrumBar::new(&mut cfg.spectrogram_gradient).home(home).show(ui).on_hover_text(
        "Hue range for audio levels. \
                 Drag the handle to change its span, the track to rotate it, or the end button to reverse it. \
                 Double-click resets.",
    );
    SpreadBar::brightness(&mut cfg.spectrogram_gradient).home(home).show(ui).on_hover_text(
        "Brightness at the low and high audio levels: 0% is black, 100% is white. \
                 A black low end blends into the spectrogram background. \
                 Double-click resets.",
    );
    SpreadBar::chroma(&mut cfg.spectrogram_gradient).home(home).show(ui).on_hover_text(
        "Saturation at the low and high audio levels: 0% is gray, 100% is the most vivid available color. \
                 Double-click resets.",
    );
    RangeBar::new(
        &mut cfg.volume_floor_db,
        &mut cfg.volume_ceiling_db,
        crate::LEVEL_MIN_DB..=crate::LEVEL_MAX_DB,
        "Level color range",
    )
    .min_span(crate::LEVEL_RANGE_MIN_SPAN)
    .display(|db| format!("{db:.0} dB"))
    .show(ui)
    .on_hover_text(
        "Audio levels assigned the first and last colors. \
                 Independent of Spectrum level range on Analyzer, which sets curve height and ring levels. \
                 Double-click resets to the full dB range.",
    );
    preview.show(ui, &cfg.spectrogram_gradient).on_hover_text(
        "Audio colors from the low level on the left to the high level on the right.",
    );
}
