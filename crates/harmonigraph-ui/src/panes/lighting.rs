//! Light and shadow for each picture: the lattice's bloom, background glow and shadows in
//! the Lattice page's Light section, and the Spiral's bloom and the Analyzer
//! and Spiral shadows on the Analyzer page. The spectrogram's own bloom sits
//! with the MIDI ribbons it lights.

use super::section;
use crate::widgets::{choice_row, ValueBar};
use crate::AppearanceDocument;
use harmonigraph_scene::{
    GlowCurve, ShadowKernel, ShadowStyle, ViewConfig, GLOW_BALLISTICS_MAX, GLOW_CURVE_SHAPE_MAX,
    GLOW_CURVE_SHAPE_MIN, GLOW_REACH_MAX, GLOW_SHADOW_MAX, GLOW_STRENGTH_MAX, SHADOW_FALLOFF_MAX,
    SHADOW_FALLOFF_MIN, SPECTRAL_SHADOW_MAX,
};

/// The shadows under the lattice's ink, last in the Light section. They stay
/// editable with glow off: they also darken the picture behind ink.
pub(super) fn lattice_shadows(ui: &mut egui::Ui, view: &mut ViewConfig) {
    let shadow = &mut view.shadow;
    shadow_group(
        ui,
        "Ring and mark shadows",
        "Audio rings, MIDI rings, melody and bass marks",
        true,
        &mut shadow.lattice_geometry,
    );
    shadow_group(
        ui,
        "Label and cross shadows",
        "Note names, tuning marks and idle crosses",
        true,
        &mut shadow.lattice_text,
    );
}

/// The Analyzer page's lighting: the Spiral's bloom, then the shadows, which
/// fall in the Analyzer and the Spiral alike.
pub(super) fn analyzer_lighting(ui: &mut egui::Ui, appearance: &mut AppearanceDocument) {
    section(ui, "Spiral", |ui| {
        ValueBar::new(&mut appearance.view.spiral_bloom, 0.0..=2.0, "Spiral bloom")
            .unit(1.0, "×")
            .show(ui)
            .on_hover_text(
                "Soft halos around bright MIDI notes in the Spiral. \
                 0 turns bloom off; \
                 1× is the reference strength.",
            );
    });
    let shadow = &mut appearance.view.shadow;
    section(ui, "Shadows", |ui| {
        shadow_group(
            ui,
            "Notes",
            "MIDI ribbons and Spiral note dots",
            false,
            &mut shadow.spectral_geometry,
        );
        shadow_group(ui, "Labels", "Note names and axis labels", false, &mut shadow.spectral_text);
    });
}

/// The background glow, first in the Light section: its reach, strength and
/// colour, what it does to the lit ink, and its clock. The note halo beside
/// it is the Bloom base on the Mappings page.
pub(super) fn glow(ui: &mut egui::Ui, view: &mut ViewConfig) {
    super::block(ui, "Background glow");
    // A share of the node's radius, the unit the shared gap and the Clearance in
    // Note read in, and measured from the same place: the reach is a distance
    // out from the node's edge exactly as the Clearance is. Eased, because the
    // bar spans two pictures rather than one range of one: the accent — a halo
    // reaching about as far as the gap to a neighbour — is the bottom eighth
    // of it, and the wash is everything above. Cubic travel gives the accent
    // half the bar, so a light meant to sit on its own node is still dialled a
    // hundredth at a time, and the far end is reachable in the same drag.
    ValueBar::new(&mut view.glow_reach, 0.0..=GLOW_REACH_MAX, "Background glow reach")
        .eased(true)
        .percent()
        .show(ui)
        .on_hover_text(
            "Distance the background glow extends beyond a node, as a percentage of its radius. \
                     Larger values blend neighboring glows. \
                     0% turns the background glow off.",
        );
    ui.add_enabled_ui(view.glow_reach > 0.0, |ui| {
            ValueBar::new(&mut view.glow_strength, 0.0..=GLOW_STRENGTH_MAX, "Background glow gain")
            .unit(1.0, "×")
                .show(ui)
                .on_hover_text(
                    "Brightness of the background glow. 0 removes the light; 1× is the reference gain.",
                );
            ValueBar::new(&mut view.glow_accumulation, 0.0..=1.0, "Overlap buildup")
                .percent()
                .show(ui)
                .on_hover_text(
                    "Controls how light builds up where note glows overlap. \
                     0% caps their combined brightness at the level set by Background glow gain. \
                     100% lets their light build up where they overlap. \
                     A single note's glow stays unchanged.",
                );
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
            ValueBar::new(&mut view.glow_attack, 0.0..=GLOW_BALLISTICS_MAX, "Background glow attack")
                .unit(1000.0, " ms").decimals(0)
                .show(ui)
                .on_hover_text(
                    "Response time for the background glow to brighten and change color after a note arrives. 0 ms responds immediately.",
                );
            ValueBar::new(&mut view.glow_release, 0.0..=GLOW_BALLISTICS_MAX, "Background glow release")
                .unit(1000.0, " ms").decimals(0)
                .show(ui)
                .on_hover_text(
                    "Response time for the background glow to fade after the node goes silent. \
                     About 37% remains after one interval; it keeps its last color as it fades. \
                     0 ms removes it immediately.",
                );
        });
}

fn shadow_group(
    ui: &mut egui::Ui,
    name: &str,
    casters: &str,
    lattice: bool,
    style: &mut ShadowStyle,
) {
    crate::widgets::label(ui, egui::RichText::new(name).strong()).on_hover_text(casters);
    choice_row(ui, "Shadow shape", &mut style.kernel, &[
        (ShadowKernel::Distance, "Contour", "Follows the outline of each shape, keeping letters and thin strokes distinct even at large widths."),
        (ShadowKernel::Gaussian, "Blur", "A soft blur of each shape. Thin strokes cast lighter shadows than thick shapes."),
    ]);
    let width_max = if lattice { GLOW_SHADOW_MAX } else { SPECTRAL_SHADOW_MAX };
    let bar = ValueBar::new(&mut style.width, 0.0..=width_max, "Shadow width");
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
    // The Gaussian has its own profile; only Contour uses this bend.
    if style.kernel.is_distance() {
        ValueBar::new(
            &mut style.falloff,
            SHADOW_FALLOFF_MIN..=SHADOW_FALLOFF_MAX,
            "Shadow falloff",
        )
        .decimals(2)
        // The curve's x is one Shadow width across, which is the span the bar
        // redistributes and the span the number is about.
        .curve(harmonigraph_scene::standoff_level)
        .show(ui)
        .on_hover_text(
            "Where inside the width the shadow spends its darkness. \
             Negative values fall early; 0 is linear; positive values fall late. \
             The curve bends in one direction throughout, without an S shape. \
             Every setting reaches zero at one Shadow width. \
             Contour shadows only.",
        );
    }
}
