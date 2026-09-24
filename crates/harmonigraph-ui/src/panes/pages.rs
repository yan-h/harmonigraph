//! The two picture settings pages, each named after the picture it sets up and
//! holding everything that picture draws, down to its light and shadow.
//!
//! One page per picture rather than a page per aspect (Lighting, Colors across
//! both): a setting is found by looking at the picture it changes, which is
//! also where a right-click on that picture lands. Colors are the exception,
//! shared by every picture, and keep a tab of their own.

use super::labels::labels_pane;
use super::lighting::{analyzer_lighting, lattice_lighting};
use super::nodes::nodes_pane;
use super::plus::plus_pane;
use super::spectral::{spectrogram_settings_pane, spectrum_settings_pane};
use super::view::view_pane;
use crate::params::ParamBackend;
use crate::PictureState;

/// The Lattice page: the whole lattice picture, read from the camera in front
/// of it inward. What is framed ([`view_pane`]), how a sounding note draws
/// ([`nodes_pane`]), the text riding it ([`labels_pane`]), what is there when
/// nothing sounds at all ([`plus_pane`]), and last the light over all of it.
pub(super) fn lattice_settings_pane(
    ui: &mut egui::Ui,
    state: &mut PictureState,
    interaction: &mut crate::Interaction,
    params: &dyn ParamBackend,
) {
    view_pane(ui, &mut state.appearance, interaction);
    nodes_pane(ui, &mut state.appearance, params);
    labels_pane(ui, state);
    plus_pane(ui, &mut state.appearance);
    lattice_lighting(ui, &mut state.appearance);
}

/// The Analyzer page: the analyzer's layout and the audio analysis every audio
/// view shares, then the spectrogram drawn inside it, then the Spiral's bloom
/// and the shadows the Analyzer and Spiral cast.
pub(super) fn analyzer_settings_pane(
    ui: &mut egui::Ui,
    state: &mut PictureState,
    interaction: &mut crate::Interaction,
    params: &dyn ParamBackend,
) {
    spectrum_settings_pane(ui, state, &mut interaction.dock, params);
    spectrogram_settings_pane(ui, state);
    analyzer_lighting(ui, &mut state.appearance);
}
