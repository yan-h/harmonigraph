//! The Mappings page: how what is played and heard maps onto what is drawn, for
//! every picture at once. MIDI notes take their color from pitch and their
//! opacity, bloom and thickness from how they are played; analyzed audio takes
//! its color from level. Shadows and each picture's own light live on its own
//! page.

use super::section;
use crate::params::{ParamBackend, ParamKey};
use crate::widgets::{
    choice_row, BendPlot, GradientPreview, RangeBar, SpectrumBar, SpreadBar, ValueBar,
};
use crate::AppearanceDocument;
use harmonigraph_scene::{
    IntensitySource, IntensityTarget, ViewConfig, BLOOM_MAX, INTENSITY_WEIGHT_MAX,
    THICKNESS_MAX_RANGE,
};

/// MIDI colors and their pitch range, then how a note's playing draws it, then
/// audio colors and their level range.
pub(super) fn color_pane(
    ui: &mut egui::Ui,
    appearance: &mut AppearanceDocument,
    params: &dyn ParamBackend,
) {
    section(ui, "MIDI note colors", |ui| {
        // The gradient above the range because it is the coarser of the two: it
        // says what the colors ARE, the range says which pitches they are spread
        // over. Both feed the one table every pitch-colored shape reads, so a
        // change here repaints the discs, the octave glyphs, the trail and the
        // piano roll together.
        spectrum_group(ui, &mut appearance.view);
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
    });
    section(ui, "Note intensity", |ui| intensity(ui, &mut appearance.view));
    section(ui, "Audio level colors", |ui| {
        crate::widgets::weak(ui, "Shared by the audio views. Lattice rings use Idle ring brightness and gray at the quiet end.");
        spectrogram_gradient_group(ui, &mut appearance.spectrum);
    });
}

/// The tooltip both groups' curve plots carry.
const BEND_HINT: &str = "Where along the range the switched-on channels spend their change. \
                 Across is the range, up is how much of the change has happened. \
                 Drag right to hold the change back for the top of the range, for example the loudest few dB. \
                 Double-click straightens it.";

fn spectrum_group(ui: &mut egui::Ui, view: &mut ViewConfig) {
    // One preset, the gradient a fresh view opens on, so the shipped look is a
    // click away after the bars below have wandered off it. Named for its
    // colors like the audio palettes, not "Default".
    crate::widgets::preset_row(ui, "Palette", &["Dusk"], |ui, menu| {
        if ui.button("Dusk").on_hover_text("Navy through violet and rose to cream").clicked() {
            view.pitch_gradient = ViewConfig::default().pitch_gradient;
            if menu {
                ui.close();
            }
        }
    });
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
    BendPlot::new(&mut view.pitch_gradient).show(ui).on_hover_text(BEND_HINT);
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
/// dial it, and the three names are the whole of what a heatmap offers before it
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
    let labels: Vec<_> = SpectrogramPreset::ALL.iter().map(|preset| preset.label()).collect();
    crate::widgets::preset_row(ui, "Palette", &labels, |ui, menu| {
        for preset in SpectrogramPreset::ALL {
            if ui.button(preset.label()).on_hover_text(preset.hint()).clicked() {
                cfg.spectrogram_gradient = preset.gradient();
                if menu {
                    ui.close();
                }
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
    BendPlot::new(&mut cfg.spectrogram_gradient).home(home).show(ui).on_hover_text(BEND_HINT);
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

/// How each note is drawn from how it is played: each of its velocity and
/// expressions routed to one display with a weight of its own, added above
/// its base (with timbre signed). The lattice and the roll
/// read the same routes, which is why it sits here rather than on either
/// picture's page.
fn intensity(ui: &mut egui::Ui, view: &mut ViewConfig) {
    fn weight<'a>(value: &'a mut f32, label: &'a str) -> ValueBar<'a> {
        ValueBar::new(value, 0.0..=INTENSITY_WEIGHT_MAX, label)
    }
    let targets = IntensityTarget::ALL.map(|target| match target {
            IntensityTarget::Off => (target, "Off", "Drives nothing."),
            IntensityTarget::Opacity => (
                target,
                "Opacity",
                "Drives how opaque the note is: the lattice's octave slices and the roll's ribbons.",
            ),
            IntensityTarget::Glow => (
                target,
                "Bloom",
                "Drives the bloom halo round lattice slices and roll ribbons, starting from Bloom base. Timbre can also reduce it.",
            ),
            IntensityTarget::Thickness => (
                target,
                "Thickness",
                "Adds thickness in multiples of the base width: Ribbon width for the roll, and the MIDI Layers bar for lattice slices. Weight 1 at full velocity or pressure adds one base width. Only timbre can thin a note below its base.",
            ),
        });
    let route = |ui: &mut egui::Ui, source: &mut IntensitySource, name: &str, hover: &str| {
        choice_row(ui, name, &mut source.target, &targets);
        ui.add_enabled_ui(source.target != IntensityTarget::Off, |ui| {
            weight(&mut source.weight, &format!("{name} weight")).show(ui).on_hover_text(format!(
                "{hover} Multiply this amount by the weight, then add it to the target's base with the other mapped sources. The final result stops at the target's limits."
            ));
        });
    };
    let intensity = &mut view.intensity;
    route(
        ui,
        &mut intensity.velocity,
        "Velocity",
        "The note-on velocity, from 0 to 1. Full velocity adds one whole weight; softer notes add less and never subtract.",
    );
    route(
        ui,
        &mut intensity.gain,
        "Gain",
        "The note's linear gain: silence adds nothing, unity (0 dB) adds one whole weight, \
             and a gain of 2 (about +6 dB) adds twice the weight. A cut adds less and never subtracts.",
    );
    route(
        ui,
        &mut intensity.pressure,
        "Pressure",
        "The note's pressure (aftertouch), from 0 unpressed to 1. Full pressure adds one whole weight; unpressed adds nothing.",
    );
    route(
        ui,
        &mut intensity.timbre,
        "Timbre",
        "Timbre is centered at 0.5, which adds nothing. Minimum timbre subtracts one whole weight; maximum timbre adds one whole weight. It can reduce a target below its base.",
    );
    // The whole of the lattice's and the roll's bloom, so it is live
    // whether or not anything is routed to Glow.
    ValueBar::new(&mut intensity.glow_base, 0.0..=BLOOM_MAX, "Bloom base")
        .unit(1.0, "×")
        .show(ui)
        .on_hover_text(
            "Soft halos around MIDI notes in the Lattice and the spectrogram's ribbons: \
                 the starting bloom, even with no mappings. Only timbre can reduce it. \
                 At base 0, mapped sources can still add bloom; 1× is the reference strength.",
        );
    ValueBar::new(&mut intensity.opacity_rest, 0.0..=1.0, "Opacity base").show(ui).on_hover_text(
        "The starting opacity, even with no mappings. Velocity, pressure and gain add above it; \
                 only timbre can reduce it. The final opacity is held between 0 and 1.",
    );
    // The ceiling matters only while a source can change the thickness.
    let disabled = "Route a source to it above to use this.";
    ui.add_enabled_ui(intensity.routes_to(IntensityTarget::Thickness), |ui| {
        let hover = "The widest a note can be drawn, as a multiple of its pane's note width. \
                         1× leaves no room to thicken; timbre can still thin it. \
                         On the lattice a slice also stops at the node's edge.";
        ValueBar::new(&mut intensity.thickness_max, THICKNESS_MAX_RANGE, "Thickness max")
            .unit(1.0, "×")
            .show(ui)
            .on_hover_text(hover)
            .on_disabled_hover_text(format!("{hover} {disabled}"));
    });
}
