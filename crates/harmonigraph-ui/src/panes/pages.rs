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
/// of it inward. Geometry, motion, labels and each light effect fold independently
/// so returning to one subject does not require opening all note or light controls.
pub(super) fn lattice_settings_pane(
    ui: &mut egui::Ui,
    state: &mut PictureState,
    interaction: &mut crate::Interaction,
    params: &dyn ParamBackend,
) {
    section(ui, "View", |ui| {
        view::camera(ui, &mut state.appearance, interaction);
        view::sevens(ui, &mut state.appearance);
    });
    section(ui, "Note layers", |ui| nodes::layers(ui, &mut state.appearance.view));
    section(ui, "Note material", |ui| nodes::material(ui, &mut state.appearance.view));
    section(ui, "Octave layout", |ui| nodes::octaves(ui, &mut state.appearance.view));
    section(ui, "Note animation", |ui| nodes::motion(ui, &mut state.appearance.view, params));
    section(ui, "Note labels", |ui| labels::labels(ui, state));
    section(ui, "Audio ring", |ui| nodes::audio_ring(ui, &mut state.appearance.view));
    plus_pane(ui, &mut state.appearance);
    section(ui, "Note bloom", |ui| lighting::note_bloom(ui, &mut state.appearance.view.note_bloom));
    section(ui, "Background glow", |ui| lighting::glow(ui, &mut state.appearance.view));
    lattice_atmosphere::settings(ui, &mut state.appearance.view);
    section(ui, "Shadows", |ui| lighting::lattice_shadows(ui, &mut state.appearance.view));
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
    ribbons_section(ui, &mut state.appearance.spectrum, &mut state.appearance.view.note_bloom);
    view_section(ui, state, &mut interaction.dock);
    analysis_section(ui, &mut state.appearance.spectrum, params);
    analyzer_lighting(ui, &mut state.appearance);
}
