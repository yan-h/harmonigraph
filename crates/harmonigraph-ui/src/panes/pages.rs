//! The two picture settings pages, each named after the picture it sets up and
//! holding everything that picture draws, down to its light and shadow.
//!
//! One page per picture rather than a page per aspect (Lighting, Colors across
//! both): a setting is found by looking at the picture it changes, which is
//! also where a right-click on that picture lands. Colors are the exception,
//! shared by every picture, and keep a tab of their own.

use super::lighting::analyzer_lighting;
use super::plus::plus_pane;
use super::spectral::{analysis_section, ribbons_section, spectrogram_section, view_section};
use super::{labels, lattice_atmosphere, lighting, nodes, section, view};
use crate::params::ParamBackend;
use crate::PictureState;

/// The Lattice page: the whole lattice picture, read from the camera in front
/// of it inward. What is framed (View), how a sounding note draws and moves
/// (Notes), what is there when nothing sounds at all ([`plus_pane`]), and last
/// the light over all of it.
pub(super) fn lattice_settings_pane(
    ui: &mut egui::Ui,
    state: &mut PictureState,
    interaction: &mut crate::Interaction,
    params: &dyn ParamBackend,
) {
    section(ui, "View", |ui| {
        view::sevens(ui, &mut state.appearance);
        view::camera(ui, &mut state.appearance, interaction);
    });
    section(ui, "Notes", |ui| {
        nodes::layers(ui, &mut state.appearance.view);
        nodes::motion(ui, &mut state.appearance.view, params);
        nodes::intensity(ui, &mut state.appearance.view);
        labels::labels(ui, state);
        nodes::octaves(ui, &mut state.appearance.view);
        nodes::audio_ring(ui, &mut state.appearance.view);
    });
    plus_pane(ui, &mut state.appearance);
    section(ui, "Light", |ui| {
        let view = &mut state.appearance.view;
        lighting::glow(ui, view);
        lattice_atmosphere::settings(ui, view);
        lighting::lattice_shadows(ui, view);
    });
}

/// The Analyzer page, most dialled first: the spectrogram's look and the MIDI
/// ribbons over it, then the analyzer's layout and its two axes, the audio
/// analysis every audio view shares, and last the Spiral's bloom and the
/// shadows the Analyzer and Spiral cast. Sorted as the Lattice page is.
pub(super) fn analyzer_settings_pane(
    ui: &mut egui::Ui,
    state: &mut PictureState,
    interaction: &mut crate::Interaction,
    params: &dyn ParamBackend,
) {
    spectrogram_section(ui, &mut state.appearance.spectrum);
    ribbons_section(ui, &mut state.appearance.spectrum);
    view_section(ui, state, &mut interaction.dock);
    analysis_section(ui, &mut state.appearance.spectrum, params);
    analyzer_lighting(ui, &mut state.appearance);
}
