//! Shared lighting effects, followed by the lattice's own glow and shadows.

use super::section;
use crate::widgets::{choice_row, ValueBar};
use crate::SharedState;
use harmonigraph_scene::{
    GlowCurve, ShadowKernel, ShadowSettings, ShadowStyle, ViewConfig, GLOW_BALLISTICS_MAX,
    GLOW_CURVE_SHAPE_MAX, GLOW_CURVE_SHAPE_MIN, GLOW_REACH_MAX, GLOW_SHADOW_MAX, GLOW_STRENGTH_MAX,
};

pub(super) fn lighting_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    ui.heading("Bloom");
    ValueBar::new(&mut state.view.bloom_strength, 0.0..=1.5, "Bloom amount")
        .unit(1.0, "×")
        .show(ui)
        .on_hover_text(
            "Soft halos around bright MIDI notes in the Lattice, Analyzer and Spiral. \
                 0 turns bloom off; \
                 1× is the reference strength.",
        );
    glow_section(ui, &mut state.view);
    shadow_groups(ui, &mut state.view.shadow);
}

fn glow_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    section(ui, "Lattice glow");
    // A share of the node's radius, the unit the two gaps and the Clearance in
    // Note read in, and measured from the same place: the reach is a distance
    // out from the node's edge exactly as the Clearance is. Eased, because the
    // bar spans two pictures rather than one range of one: the accent — a halo
    // reaching about as far as the gap to a neighbour — is the bottom eighth
    // of it, and the wash is everything above. Cubic travel gives the accent
    // half the bar, so a light meant to sit on its own node is still dialled a
    // hundredth at a time, and the far end is reachable in the same drag.
    ValueBar::new(&mut view.glow_reach, 0.0..=GLOW_REACH_MAX, "Glow reach")
        .eased(true)
        .percent()
        .show(ui)
        .on_hover_text(
            "Distance the glow extends beyond a node, as a percentage of its radius. \
                 Larger values blend neighboring glows. \
                 0% turns glow off.",
        );
    ui.add_enabled_ui(view.glow_reach > 0.0, |ui| {
        ValueBar::new(&mut view.glow_strength, 0.0..=GLOW_STRENGTH_MAX, "Glow gain")
        .unit(1.0, "×")
            .show(ui)
            .on_hover_text("Brightness of the lattice glow. 0 removes the light; 1× is the reference gain.");
        ValueBar::new(
            &mut view.glow_curve.shape,
            GLOW_CURVE_SHAPE_MIN..=GLOW_CURVE_SHAPE_MAX,
            "Falloff curve",
        )
        .decimals(2)
        .magnet(0.0, 0.15)
        .display(|shape| format!("{shape:+.2}"))
        .curve(|shape, p| GlowCurve { shape }.sample(p))
        .show(ui)
        .on_hover_text(
            "Light falloff from node center to outer edge. \
                 0 falls evenly; positive values fade near the center; negative values hold brightness until the edge. \
                 The line previews the falloff.",
        );
        // What colour the light comes out, between the amount of it and the
        // Shadow under it. "Color smoothing" and not "Spread": under this heading,
        // beside a Reach that is about distance, a "spread" reads as how far
        // the light goes, and this moves no light at all. It reads as a
        // percentage because it is a SHARE — of a whole turn — and not a
        // distance.
        ValueBar::new(&mut view.glow_blend, 0.0..=1.0, "Color smoothing")
            .percent()
            .show(ui)
            .on_hover_text(
                "Mix the MIDI octave and melody/bass colors around each node. \
                 0% keeps separate colored arcs; \
                 100% makes one average color. \
                 Audio-ring colors do not feed the glow.",
            );
    });
    ui.add_enabled_ui(view.glow_reach > 0.0, |ui| {
        // The INK's own share of the light, where a Shadow depth says the
        // ground's: one question asked twice, and the answers are free of each
        // other on purpose — a dark pool with a tinted ring in it is a picture
        // no single coupled dial can name. Only the LIT ink is dialled, the
        // rest of the lattice always taking the whole field, for the reason the
        // hover text gives.
        ValueBar::new(&mut view.glow_wash, 0.0..=1.0, "Light on notes")
            .percent()
            .show(ui)
            .on_hover_text(
                "Amount of glow laid over active rings and marks. \
                 0% preserves their original colors; \
                 100% blends them into the surrounding light. \
                 Idle shapes always receive the full glow.",
            );
        // The light's own clock, last, under everything it shapes. Its own pair
        // and not the note Fade in Note, because a halo is the slow part of the
        // picture: on the layers' envelopes it flickers with the marks, which
        // are meant to be fast.
        ValueBar::new(&mut view.glow_attack, 0.0..=GLOW_BALLISTICS_MAX, "Glow attack")
            .unit(1000.0, " ms").decimals(0)
            .show(ui)
            .on_hover_text(
                "Response time for the glow to brighten and change color after a note arrives. 0 ms responds immediately.",
            );
        ValueBar::new(&mut view.glow_release, 0.0..=GLOW_BALLISTICS_MAX, "Glow release")
            .unit(1000.0, " ms").decimals(0)
            .show(ui)
            .on_hover_text(
                "Time for the glow to fade after the node goes silent. \
                 It keeps its last color as it fades. \
                 0 ms removes it immediately.",
            );
    });
}

/// Shadows remain editable with glow off: they also darken the picture behind ink.
fn shadow_groups(ui: &mut egui::Ui, shadow: &mut ShadowSettings) {
    section(ui, "Shadows");
    shadow_group(
        ui,
        "Lattice rings and marks",
        "Audio rings, MIDI rings, melody and bass marks",
        true,
        &mut shadow.lattice_geometry,
    );
    shadow_group(
        ui,
        "Lattice labels and crosses",
        "Note names, tuning marks and idle crosses",
        true,
        &mut shadow.lattice_text,
    );
    shadow_group(
        ui,
        "Analyzer and Spiral notes",
        "MIDI ribbons and Spiral note dots",
        false,
        &mut shadow.spectral_geometry,
    );
    shadow_group(
        ui,
        "Analyzer and Spiral labels",
        "Note names and axis labels",
        false,
        &mut shadow.spectral_text,
    );
}

fn shadow_group(
    ui: &mut egui::Ui,
    name: &str,
    casters: &str,
    lattice: bool,
    style: &mut ShadowStyle,
) {
    ui.label(egui::RichText::new(name).strong()).on_hover_text(casters);
    choice_row(ui, "Shadow shape", &mut style.kernel, &[
        (ShadowKernel::Distance, "Contour", "Follows the outline of each shape, keeping letters and thin strokes distinct even at large widths."),
        (ShadowKernel::Gaussian, "Blur", "A soft blur of each shape. Thin strokes cast lighter shadows than thick shapes."),
    ]);
    let bar = ValueBar::new(&mut style.width, 0.0..=GLOW_SHADOW_MAX, "Shadow width");
    let (bar, hint) = if lattice {
        (bar.percent(), "Shadow width as a percentage of the node radius. Scales with lattice zoom. 0% removes the shadow.")
    } else {
        (bar.unit(harmonigraph_render::SPECTRAL_WIDTH_POINTS, " pt").decimals(2), "Shadow width in screen points. Stays constant when you zoom frequency. 0 pt removes the shadow.")
    };
    bar.show(ui).on_hover_text(hint);
    ValueBar::new(&mut style.depth, 0.0..=1.0, "Shadow darkness").percent().show(ui).on_hover_text(
        "Maximum darkening beneath this group. \
                 0% removes the shadow; \
                 100% turns the area beneath solid shapes black. \
                 Thin strokes may cast lighter shadows.",
    );
}
