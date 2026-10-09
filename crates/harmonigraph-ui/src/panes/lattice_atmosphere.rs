//! Independent texture and material stages in the lattice's background glow.

use harmonigraph_scene::{
    AtmosphereSettings, LatticeMaterial, NEBULA_SCALE_MAX, NEBULA_SCALE_MIN, NEBULA_SPEED_MAX,
    NEBULA_SPEED_MIN, PIGMENT_REACH_MAX,
};

pub(super) fn settings(ui: &mut egui::Ui, view: &mut harmonigraph_scene::ViewConfig) {
    let glow_enabled = view.glow_reach > 0.0 && view.glow_strength > 0.0;
    let settings = &mut view.atmosphere;
    use crate::widgets::{choice_row, ValueBar};

    super::section(ui, "Glow pattern", |ui| {
        if !glow_enabled {
            crate::widgets::weak(
                ui,
                "Set Background glow reach and gain above zero to see these effects.",
            );
        }
        ui.add_enabled_ui(glow_enabled, |ui| {
            ValueBar::new(&mut settings.texture_depth, 0.0..=1.0, "Pattern contrast")
                .percent().show(ui).on_hover_text("Pattern contrast before the material shapes the light. 0% restores smooth halos; material settings remain active.");
            ValueBar::new(&mut settings.texture_scale, NEBULA_SCALE_MIN..=NEBULA_SCALE_MAX, "Pattern size")
                .eased(true)
                .unit(1.0, "×")
                .show(ui)
                .on_hover_text("Larger values make broader patterns. The material can reshape this pattern.");
            multiplier(ui, &mut settings.texture_speed, "Pattern speed", NEBULA_SPEED_MIN..=NEBULA_SPEED_MAX)
                .on_hover_text("Pattern motion before the material. 0 freezes this motion; notes and the material can still change the picture.");
        crate::widgets::button_row(ui, |ui| {
            if ui.button("Reset pattern").on_hover_text("Reset only the pattern contrast, size and speed.").clicked() {
                let fresh = AtmosphereSettings::default();
                settings.texture_depth = fresh.texture_depth;
                settings.texture_scale = fresh.texture_scale;
                settings.texture_speed = fresh.texture_speed;
            }
        });
    });
    });
    super::section(ui, "Glow material", |ui| {
        if !glow_enabled {
            crate::widgets::weak(
                ui,
                "Set Background glow reach and gain above zero to see these effects.",
            );
        }
        ui.add_enabled_ui(glow_enabled, |ui| {
        choice_row(ui, "Material", &mut settings.material_style, &[
            (LatticeMaterial::None, "None", "Keep the textured light as it is; preserves material settings"),
            (LatticeMaterial::Watercolor, "Watercolor", "Overlapping washes of the textured note light"),
            (LatticeMaterial::VelvetScales, "Scales", "Soft overlapping scallops carrying the note light"),
            (LatticeMaterial::Stars, "Stars", "Drifting stars colored by the note light"),
        ]);
        if settings.material_style != LatticeMaterial::None {
            ValueBar::new(&mut settings.material_amount, 0.0..=1.0, "Material amount")
                .percent().show(ui).on_hover_text("How strongly the material reshapes the textured light. 0% bypasses the material while preserving the texture.");
            if settings.material_style == LatticeMaterial::Stars {
                super::material::stars_resolution(ui, &mut settings.stars);
            }
            ValueBar::new(&mut settings.material_shadow_pickup, 0.0..=1.0, "Dark pickup")
                .percent().show(ui).on_hover_text("Dark pigment behind unlit ring segments, picked up by the material. Fades as each segment lights up. Actual ring and label shadows keep their own settings.");
            ValueBar::new(&mut settings.material_color_pickup, 0.0..=1.0, "Color pickup")
                .percent().show(ui).on_hover_text("Pitch-colored pigment behind lit ring segments, picked up by the material. Follows each segment’s activation and Bloom brightness. Spreads by distance around the segment, including its ends. 0% disables colored pickup.");
            ValueBar::new(&mut settings.pigment_reach, 0.0..=PIGMENT_REACH_MAX, "Pigment reach")
                .percent().show(ui).on_hover_text("Distance pigment reaches from each ring segment, as a percentage of the node radius. Grows the source band and its soft feather together. 0% disables dark and colored pickup; actual shadows are unchanged.");
            // Stars put their look first: size and spacing matter more than
            // how they drift.
            let stars = settings.material_style == LatticeMaterial::Stars;
            if stars {
                super::block(ui, "Appearance");
                super::material::stars(ui, &mut settings.stars, harmonigraph_scene::LATTICE_STAR_SIZE_SCALE);
            }
            super::block(ui, "Motion");
            if stars {
                super::material::stars_motion(ui, &mut settings.stars);
            }
            let speed = (!stars).then_some(&mut settings.material_speed);
            crate::widgets::drift(ui, &mut settings.material_direction, speed);
            match settings.material_style {
                LatticeMaterial::Watercolor => {
                    super::block(ui, "Appearance");
                    super::material::watercolor(ui, &mut settings.material_settings);
                }
                LatticeMaterial::VelvetScales => {
                    super::block(ui, "Appearance");
                    super::material::velvet(ui, &mut settings.material_settings);
                }
                LatticeMaterial::Stars | LatticeMaterial::None => {},
            }
        }
        crate::widgets::button_row(ui, |ui| {
            if ui.button("Reset material").on_hover_text("Reset only the material choice, amount, motion and geometry controls.").clicked() {
                let fresh = AtmosphereSettings::default();
                settings.material_style = fresh.material_style;
                settings.material_amount = fresh.material_amount;
                settings.material_shadow_pickup = fresh.material_shadow_pickup;
                settings.material_color_pickup = fresh.material_color_pickup;
                settings.pigment_reach = fresh.pigment_reach;
                settings.material_settings = fresh.material_settings;
                settings.stars = fresh.stars;
                settings.material_speed = fresh.material_speed;
                settings.material_direction = fresh.material_direction;
            }
        });
    });
    });
}

fn multiplier(
    ui: &mut egui::Ui,
    value: &mut f32,
    label: &str,
    range: std::ops::RangeInclusive<f32>,
) -> egui::Response {
    crate::widgets::ValueBar::new(value, range, label).unit(1.0, "×").show(ui)
}
