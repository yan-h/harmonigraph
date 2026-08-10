//! The Display tab's Color & light section: what everything on screen is
//! painted with. The gradient and the Color range write the one table every
//! pitch-colored shape reads — the lattice's discs and octave glyphs, the
//! trail, and the Analyzer's note ribbons — and Bloom is the light riding on
//! the result, post-process on both pictures at once.
//!
//! First section in the tab because it is the widest-scope thing in it and the
//! most reached for: nothing here belongs to one layer, one pane or one
//! picture. A played note's own layers are [`super::nodes`], the text on them
//! [`super::labels`], and the analyzer's heatmap gradient stays with the
//! analyzer — that one is level->color and read by nothing else, where this is
//! the shared table.

use crate::params::{ParamBackend, ParamKey};
use crate::widgets::{GradientPreview, SpectrumBar, SpreadBar, ValueBar};
use crate::SharedState;
use harmonigraph_scene::ViewConfig;

/// The colors, then the light: the gradient group, the pitch span it is spread
/// over, and the halo the result is bloomed with.
///
/// No leading heading, unlike the Nodes and View bodies: this section is one
/// group, and the only name it could take is the fold-out header's own.
pub(super) fn color_pane(ui: &mut egui::Ui, state: &mut SharedState, params: &dyn ParamBackend) {
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
        "The pitch span the color gradient covers: the low end takes the \
         gradient's first color, the high end its last. Drag either end, or \
         drag between them to slide the whole range.\n\nNot the same thing as \
         the Analyzer's Pitch range, which is the slice of the spectrum on \
         show: this one moves no picture, it only decides which pitches get \
         which colors.",
    );
    bloom_bar(ui, &mut state.view);
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
            "Soft halo around bright notes, on the lattice's nodes and on the \
             Analyzer's note ribbons alike; 0 turns the post-process off",
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
        "The pitch->color spectrum: how far round the color circle the pitch \
         range walks, out of the whole turn the bar stands for. The hues it \
         takes fill from the left, low note first; the ones it does not are \
         dimmed. The track is hue alone — the Brightness and Saturation bars \
         below move the picture above, not the arc. Drag the handle to widen or \
         narrow it, drag the track to turn the circle under it, double-click \
         to reset. The button past the right end runs the whole thing the \
         other way round the circle.",
    );
    SpreadBar::brightness(&mut view.pitch_gradient).show(ui).on_hover_text(
        "The stretch of brightness the pitch range spends, in CIELab L*: the \
         two numbers are the bottom of the pitch range and the top, in that \
         order, so a picture with its bright end at the bottom reads out \
         backwards. Drag either end to move it, drag between them to slide the \
         whole stretch brighter or darker, drag one end past the other to swap \
         which end is bright, double-click to reset. Closing the two together \
         makes every note exactly as bright as every other and leaves hue to \
         carry the pitch alone.",
    );
    SpreadBar::chroma(&mut view.pitch_gradient).show(ui).on_hover_text(
        "The stretch of color the pitch range spends, each end one \
         colorfulness whatever hue that note lands on — 100% is as \
         vivid as the screen goes without distorting the color, 0 is grey. The \
         two numbers are the bottom of the pitch range and the top, in that \
         order, so a picture with its vivid end at the bottom reads out \
         backwards. Drag either end to move it, drag between them to slide the \
         whole stretch, drag one end past the other to swap which end is \
         vivid, double-click to reset. Closing the two together gives every \
         note the same share of the color available to it.",
    );
    preview.show(ui, &view.pitch_gradient).on_hover_text(
        "The gradient itself, low note on the left: every one of the six \
         numbers the bars below carry, composed into the colors every \
         pitch-colored shape is drawn with — the lattice's discs and octave \
         glyphs, the trail, and the note ribbons in the Analyzer. A picture \
         rather than a control — the three bars under it are what move it.",
    );
}
