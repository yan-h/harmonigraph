//! Light and shadow for each picture: the lattice's bloom, background glow and shadows in
//! separate Lattice sections, and the Spiral's bloom and the Analyzer
//! and Spiral shadows on the Analyzer page. Note bloom sits with the MIDI
//! ribbons it lights.

use super::section;
use crate::widgets::ValueBar;
use crate::AppearanceDocument;
use harmonigraph_scene::{
    AtmosphereSettings, GlowCurve, ShadowStyle, ViewConfig, BREATH_SPEED_MAX, BREATH_SPEED_MIN,
    GLOW_BALLISTICS_MAX, GLOW_CURVE_SHAPE_MAX, GLOW_CURVE_SHAPE_MIN, GLOW_REACH_MAX,
    GLOW_SHADOW_MAX, GLOW_STRENGTH_MAX, SPECTRAL_SHADOW_MAX,
};
use harmonigraph_scene::{
    GLOW_ACCUMULATION_RANGE, GLOW_BLEND_RANGE, GLOW_WASH_RANGE, HIDE_BEHIND_RANGE,
    NOTE_BLOOM_RANGE, SPIRAL_BLOOM_RANGE,
};

/// The shadows under the lattice's ink, in their own section. They stay
/// editable with glow off: they also darken the picture behind ink.
pub(super) fn lattice_shadows(ui: &mut egui::Ui, view: &mut ViewConfig) {
    let shadow = &mut view.shadow;
    // Hide behind sits with the ring and mark shadows, by their Darkness: it
    // spends the same field that group's Width shapes.
    shadow_group(
        ui,
        "Ring and mark shadows",
        "Audio rings, MIDI rings, melody and bass marks",
        true,
        false,
        &mut shadow.lattice_geometry,
        Some(&mut view.hide_behind),
    );
    shadow_group(
        ui,
        "Label and cross shadows",
        "Note names, tuning marks and idle crosses",
        true,
        true,
        &mut shadow.lattice_text,
        None,
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
        note_surround(ui, &mut appearance.spectrum);
        shadow_group(
            ui,
            "Labels",
            "Analyzer note names and axis labels, and Spiral note names",
            false,
            true,
            &mut shadow.spectral_text,
            None,
        );
    });
}

/// The backdrop cut round every MIDI ribbon and Spiral dot: how much of the picture it takes
/// next to a note, how far and along what curve it fades, and what fills it.
fn note_surround(ui: &mut egui::Ui, cfg: &mut crate::SpectrumConfig) {
    super::block(ui, "Note surround", |ui| {
        ValueBar::new(&mut cfg.note_cut, 0.0..=1.0, "Backdrop cut")
            .percent()
            .show(ui)
            .on_hover_text(
                "How much of the spectrogram is taken away next to a note, so the note \
                 holds over bright energy as well as over dark. 0% turns the surround off. \
                 Over black it changes nothing but the fill.",
            );
        ValueBar::new(
            &mut cfg.note_cut_reach,
            harmonigraph_render::SURROUND_REACH_RANGE,
            "Cut reach",
        )
        .decimals(1)
        .unit(1.0, " pt")
        .show(ui)
        .on_hover_text(
            "How far out from a note the cut fades away. \
                 Longer is softer, and takes more of the gap between close notes.",
        );
        ValueBar::new(
            &mut cfg.note_cut_shape,
            harmonigraph_render::SURROUND_SHAPE_RANGE,
            "Cut shape",
        )
        .percent()
        .magnet(0.0, 0.03)
        .curve(harmonigraph_render::surround_fade)
        .show(ui)
        .on_hover_text(
            "The curve the cut fades along. 0% is a straight line; \
                 -100% drops steeply at the note and leaves a long soft tail; \
                 100% holds near the note and drops steeply at the far end.",
        );
        ValueBar::new(&mut cfg.note_cut_fill, 0.0..=1.0, "Cut fill")
            .percent()
            .show(ui)
            .on_hover_text(
                "What replaces the spectrogram the cut takes away. \
                 0% leaves it black; 100% fills it with the note's own hue.",
            );
        ValueBar::new(&mut cfg.note_fill_lightness, 0.0..=100.0, "Fill lightness")
            .decimals(0)
            .show(ui)
            .on_hover_text(
                "Lightness (L*) of the fill, drawn in each note's own hue. \
                 0 is black. Notes stand out most where this is far from their own lightness.",
            );
        ValueBar::new(&mut cfg.note_halo_blend, 0.0..=1.0, "Halo blend")
            .percent()
            .show(ui)
            .on_hover_text(
                "How smoothly the cuts of neighbouring notes merge. \
                 0% shows a sharp crease along the line between two notes; \
                 higher rounds it out and darkens narrow gaps a little more.",
            );
    })
    .response
    .on_hover_text("MIDI ribbons and Spiral note dots");
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
        super::block(ui, "Glow response", |ui| {
            crate::widgets::response(
                ui,
                &mut view.glow_attack,
                &mut view.glow_release,
                GLOW_BALLISTICS_MAX,
                ["Background glow attack", "Background glow release"],
                1000.0,
            );
        });
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
    text: bool,
    style: &mut ShadowStyle,
    hide_behind: Option<&mut f32>,
) {
    super::block(ui, name, |ui| {
        let width_max = if lattice { GLOW_SHADOW_MAX } else { SPECTRAL_SHADOW_MAX };
        crate::widgets::shadow(ui, style, width_max, lattice, text);
        // Under the diagram rather than in its value column: the preview draws
        // the visible shadow, which this does not move, and the column holds
        // three rows. Greyed at Width 0, which packs no field for it to spend.
        if let Some(hide_behind) = hide_behind {
            ui.add_enabled_ui(style.width > 0.0, |ui| {
                ValueBar::new(hide_behind, HIDE_BEHIND_RANGE, "Hide behind")
                    .percent()
                    .show(ui)
                    .on_hover_text(
                        "How much a ring in front hides the rings, marks, names and crosses \
                         behind it, whatever the shadow's Darkness. 0% hides nothing; a Width \
                         of 0 turns it off with the shadow.",
                    );
            });
        }
    })
    .response
    .on_hover_text(casters);
}
