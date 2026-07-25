//! The Nodes pane: how a sounding note is drawn — its core mark, the octave
//! ring around it, the melody/bass marks on the outer held notes, and the
//! color and fade the whole node wears. Everything here is the *played note*;
//! the surrounding structure and overlays live in [`super::scene`].

use super::{param_bar, section};
use crate::params::{ParamBackend, ParamKey};
use crate::widgets::{choice_row, RangeBar, ValueBar};
use crate::SharedState;
use lattice_scene::{NodeStyle, ViewConfig};

/// The sounding-note controls, top to bottom as the note reads outward: the
/// Core mark at its center, the Octaves ring around it, the melody/bass marks
/// on the outer notes, then the Color it's tinted and the Fade it lingers on
/// release. Scrolls so the full list is reachable in a short pane.
pub(super) fn nodes_pane(ui: &mut egui::Ui, state: &mut SharedState, params: &dyn ParamBackend) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            core_section(ui, &mut state.view);
            octaves_section(ui, &mut state.view);
            melody_bass_section(ui, &mut state.view);
            color_section(ui, params);
            fade_section(ui, params);
        });
}

/// Core: the mark at a sounding node's center. One continuous shape sized by
/// the radius (0 = off, like Bloom) and morphed by Solidity from a soft glow
/// (0) to the classic solid orb (1), painted per the Style row. Independent
/// of the Octaves layer.
fn core_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    ui.heading("Core");
    ValueBar::new(&mut view.core_radius, 0.0..=0.9, "Radius")
        .show(ui)
        .on_hover_text(
            "Core size (disc and glow together); 0 turns it off, \
             0.46 is the classic disc",
        );
    ui.add_enabled_ui(view.core_radius > 0.0, |ui| {
        ValueBar::new(&mut view.core_solidity, 0.0..=1.0, "Solidity")
            .show(ui)
            .on_hover_text(
                "0 = a soft glow, 1 = the classic solid orb; in \
                 between the disc fades in over its glow and its \
                 edge crisps",
            );
        // Switchable paints (idle nodes look the same in all).
        // Steady is a calm solid disc blending the sounding octaves'
        // colors; the rest are field styles — Vortex the gas look,
        // Checker and Spiral patterns on the sphere. The paint
        // dissolves with the disc toward the glow end.
        choice_row(
            ui,
            "Style",
            &mut view.node_style,
            &[
                (NodeStyle::Steady, "Steady", ""),
                (NodeStyle::Vortex, "Vortex", ""),
                (NodeStyle::Checker, "Checker", ""),
                (NodeStyle::Spiral, "Spiral", ""),
            ],
        );
    });
}

/// Octaves: which octaves of the pitch class are sounding, shown as glyphs at
/// each note's absolute-pitch angle within a radial band. Independent of the
/// Core.
fn octaves_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    section(ui, "Octaves");
    // No on/off: the layer is what says which octaves are sounding, which
    // is the node's whole outer half, and Band already reaches every size
    // from a hairline to the quad edge.
    //
    // Inner and outer radius are one control: the band is the ring between
    // them. Drag either edge, or drag between to slide the ring at a fixed
    // width; the min span keeps it from collapsing.
    ui.label("Band");
    RangeBar::new(&mut view.outer_inner, &mut view.outer_outer, 0.0..=1.0)
        .min_span(0.05)
        .show(ui)
        .on_hover_text(
            "The octave band's inner and outer radius. Inner 0 reaches the \
             node center (pie wedges); drag between the handles to move the \
             whole ring in or out.",
        );
    ValueBar::new(&mut view.outer_solidity, 0.0..=1.0, "Solidity")
        .show(ui)
        .on_hover_text(
            "0 = soft glowy octave marks, 1 = the crisp classic \
             shapes; only softens the glyph edges, shapes and \
             angles stay put",
        );
    // One padding for the whole layer: between sectors, and
    // between the band and the melody/bass rings.
    ValueBar::new(&mut view.outer_gap, 0.0..=0.4, "Gap")
        .show(ui)
        .on_hover_text(
            "Padding inside the octave layer: between one octave \
             and the next, and between the band and the \
             melody/bass rings. 0 closes the octaves into a solid \
             annulus and seats the rings against it. Wide values \
             push the bass ring in toward the core -- raise Band \
             inner to make room",
        );
    // Backdrop: draw the silent octaves faintly so a lone octave
    // still reads as a whole note.
    ValueBar::new(&mut view.outer_backdrop, 0.0..=1.0, "Backdrop")
        .show(ui)
        .on_hover_text(
            "Complete the octave ring: draw the silent octaves \
             faintly behind the sounding sectors, so a lone octave \
             still reads as a whole note. 0 = off",
        );
}

/// Melody / bass: mark the outer held notes so a chord's top and bottom line
/// read at a glance.
fn melody_bass_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    section(ui, "Melody / bass");
    // Two boxes, not a four-way row: the marks are independent, they are
    // told apart by radius rather than by hue, and a note that is at once
    // the highest and the lowest -- a lone held note, or a chord whose top
    // and bottom share a pitch class -- simply gets both.
    ui.checkbox(&mut view.mark_melody, "Melody")
        .on_hover_text("Ring the highest held note, just inside the octave band");
    ui.checkbox(&mut view.mark_bass, "Bass")
        .on_hover_text("Ring the lowest held note, just outside the octave band");
    // The marks are full rings bracketing the octave band (melody
    // inside, bass outside), each slit either side of the octave
    // responsible so that stretch reads as its own piece.
    ui.add_enabled_ui(view.mark_melody || view.mark_bass, |ui| {
        ValueBar::new(&mut view.mark_thickness, 0.0..=0.3, "Thickness")
            .show(ui)
            .on_hover_text(
                "How thick both mark rings are, in the same units as \
                 the band radii and Gap. 0 turns the rings off; thick \
                 values grow the bass ring in over the core, so raise \
                 Band inner to make room",
            );
    });
}

/// A MIDI note as a key name and octave, for the color-range readout — "C1",
/// "C8" — so the ends read as pitches rather than bare numbers.
fn pitch_readout(midi: f32) -> String {
    let n = midi.round() as i32;
    let name = super::KEY_NAMES[n.rem_euclid(12) as usize];
    format!("{name}{}", lattice_core::notes::display_octave_of(n))
}

/// Color: the pitch->color gradient endpoints the pitch-colored channels map
/// through — the darkest pitch and the brightest, as one two-handle range.
fn color_section(ui: &mut egui::Ui, params: &dyn ParamBackend) {
    section(ui, "Color");
    ui.label("Pitch range");
    super::param_range_bar(
        ui,
        params,
        ParamKey::DarkestPitch,
        ParamKey::BrightestPitch,
        0.0..=120.0,
        crate::PITCH_RANGE_MIN_SPAN,
        pitch_readout,
    )
    .on_hover_text(
        "The pitch span the color gradient covers: the low end takes the \
         darkest color, the high end the brightest. Drag either end, or drag \
         between them to slide the whole range.",
    );
}

/// Fade: how long a released note lingers. One time for the whole node —
/// core, octave glyphs, and melody/bass marks — rather than one per layer, so
/// a release reads as a single gesture instead of pieces of the node going
/// dark at different moments.
fn fade_section(ui: &mut egui::Ui, params: &dyn ParamBackend) {
    section(ui, "Fade");
    param_bar(ui, params, ParamKey::Fade).on_hover_text(
        "Seconds a released note keeps fading — the pitch class core, \
         the octave glyphs, and the melody/bass marks together. 0 cuts \
         notes off the moment they're released",
    );
}
