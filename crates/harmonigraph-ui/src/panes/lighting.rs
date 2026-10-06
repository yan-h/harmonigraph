//! Light and shadow for each picture: the lattice's bloom, background glow and shadows in
//! separate Lattice sections, and the Spiral's bloom and the Analyzer
//! and Spiral shadows on the Analyzer page. Note bloom sits with the MIDI
//! ribbons it lights.

use super::section;
use crate::widgets::{choice_row, ValueBar};
use crate::AppearanceDocument;
use harmonigraph_scene::{
    AtmosphereSettings, GlowCurve, ShadowKernel, ShadowStyle, ViewConfig, BREATH_SPEED_MAX,
    BREATH_SPEED_MIN, GLOW_BALLISTICS_MAX, GLOW_CURVE_SHAPE_MAX, GLOW_CURVE_SHAPE_MIN,
    GLOW_REACH_MAX, GLOW_SHADOW_MAX, GLOW_STRENGTH_MAX, SPECTRAL_SHADOW_MAX,
};
use harmonigraph_scene::{
    GLOW_ACCUMULATION_RANGE, GLOW_BLEND_RANGE, GLOW_WASH_RANGE, NOTE_BLOOM_RANGE,
    SPIRAL_BLOOM_RANGE,
};

/// The shadows under the lattice's ink, in their own section. They stay
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
        ValueBar::new(&mut appearance.view.spiral_bloom, SPIRAL_BLOOM_RANGE, "Spiral bloom")
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
        shadow_group(
            ui,
            "Labels",
            "Analyzer note names and axis labels, and Spiral note names",
            false,
            &mut shadow.spectral_text,
        );
    });
}

/// The same global note bloom is editable beside either picture it affects.
pub(crate) fn note_bloom(ui: &mut egui::Ui, strength: &mut f32) {
    ValueBar::new(strength, NOTE_BLOOM_RANGE, "Note bloom")
        .unit(1.0, "×")
        .show(ui)
        .on_hover_text(
            "Soft halos around bright notes. Shared by the Lattice and MIDI ribbons in the Analyzer. \
             Changing this control updates both pictures. 0 turns bloom off; 1× is the reference strength.",
        );
    crate::widgets::weak(ui, "Shared by Lattice and Analyzer MIDI ribbons.");
}

/// The background glow: its reach, strength, colour, wash and clock.
pub(super) fn glow(ui: &mut egui::Ui, view: &mut ViewConfig) {
    // Stored in the node's quad uv, as the Gap is, and measured out from the
    // node's outermost drawn edge; read out in true node radii
    // (`QUAD_UV_PERCENT`, uv 1.0 = 180%), as its hover says. Eased, because the bar spans two pictures rather
    // than one range of one: the accent — a halo reaching about as far as the
    // gap to a neighbour — is the bottom eighth of it, and the wash is
    // everything above. Cubic travel gives the accent
    // half the bar, so a light meant to sit on its own node is still dialled a
    // hundredth at a time, and the far end is reachable in the same drag.
    ValueBar::new(&mut view.glow_reach, 0.0..=GLOW_REACH_MAX, "Background glow reach")
        .eased(true)
        .unit(super::QUAD_UV_PERCENT, "%")
        .decimals(1)
        .show(ui)
        .on_hover_text(
            "Distance the background glow extends beyond a node, as a percentage of its radius. \
                     Larger values blend neighboring glows. \
                     0% turns the background glow off.",
        );
    ui.add_enabled_ui(view.glow_reach > 0.0, |ui| {
        // The glow target's share of the lattice's resolution. A lower one is
        // cheaper on the GPU and softens the halo, which is the trade to judge.
        choice_row(
            ui,
            "Glow resolution",
            &mut view.glow_resolution,
            &[
                (2, "1/2", "Glow drawn at half the lattice's resolution: the sharpest halo, at the most GPU time."),
                (3, "1/3", "A third of the lattice's resolution: less GPU time and a softer halo."),
                (4, "1/4", "A quarter of the lattice's resolution: the least GPU time and the softest halo."),
            ],
        );
        ValueBar::new(&mut view.glow_strength, 0.0..=GLOW_STRENGTH_MAX, "Background glow gain")
            .unit(1.0, "×")
            .show(ui)
            .on_hover_text(
                "Brightness of the background glow. 0 removes the light; 1× is the reference gain.",
            );
    });
    // Everything below shapes a light that draws only at a Reach and a gain
    // both above 0, so either at 0 greys it, as it does the breathing.
    ui.add_enabled_ui(view.glow_reach > 0.0 && view.glow_strength > 0.0, |ui| {
            ValueBar::new(&mut view.glow_accumulation, GLOW_ACCUMULATION_RANGE, "Overlap buildup")
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
            ValueBar::new(&mut view.glow_blend, GLOW_BLEND_RANGE, "Color smoothing")
                .percent()
                .show(ui)
                .on_hover_text(
                    "Mix the MIDI octave and melody/bass colors around each node. \
                     0% keeps separate colored arcs; \
                     100% makes one average color. \
                     Audio-ring colors do not feed the glow.",
                );
        });
    ui.add_enabled_ui(view.glow_reach > 0.0 && view.glow_strength > 0.0, |ui| {
        // The INK's own share of the light, where a Shadow depth says the
        // ground's: one question asked twice, and the answers are free of each
        // other on purpose — a dark pool with a tinted ring in it is a picture
        // no single coupled dial can name. Only the LIT ink is dialled, the
        // rest of the lattice always taking the whole field, for the reason the
        // hover text gives.
        ValueBar::new(&mut view.glow_wash, GLOW_WASH_RANGE, "Light on notes")
            .percent()
            .show(ui)
            .on_hover_text(
                "Amount of glow laid over active rings and marks. \
                     0% preserves their original colors; \
                     100% blends them into the surrounding light. \
                     Idle shapes always receive the full glow.",
            );
        super::block(ui, "Glow response");
        crate::widgets::response(
            ui,
            &mut view.glow_attack,
            &mut view.glow_release,
            GLOW_BALLISTICS_MAX,
            ["Background glow attack", "Background glow release"],
            1000.0,
        );
    });
    ui.add_enabled_ui(view.glow_reach > 0.0 && view.glow_strength > 0.0, |ui| {
        ValueBar::new(&mut view.atmosphere.breath_amount, 0.0..=1.0, "Breathing depth")
            .percent().show(ui).on_hover_text("Brightness variation in the background glow, independent of texture and material. 0% keeps it steady.");
        ValueBar::new(&mut view.atmosphere.breath_speed, BREATH_SPEED_MIN..=BREATH_SPEED_MAX, "Breathing speed")
            .unit(1.0, "×").show(ui).on_hover_text("1× is the original slow breathing. 0 keeps the halo at its normal brightness.");
        crate::widgets::button_row(ui, |ui| {
            if ui.button("Reset breathing").clicked() {
                let fresh = AtmosphereSettings::default();
                view.atmosphere.breath_amount = fresh.breath_amount;
                view.atmosphere.breath_speed = fresh.breath_speed;
            }
        });
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
    crate::widgets::shadow(ui, style, width_max, lattice);
}
