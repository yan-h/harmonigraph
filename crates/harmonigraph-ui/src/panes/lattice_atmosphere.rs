//! Independent texture and material stages in the lattice's background glow.

use harmonigraph_scene::{
    AtmosphereSettings, LatticeMaterial, NEBULA_SCALE_MAX, NEBULA_SCALE_MIN, NEBULA_SPEED_MAX,
    NEBULA_SPEED_MIN,
};

pub(super) fn settings(ui: &mut egui::Ui, view: &mut harmonigraph_scene::ViewConfig) {
    let glow_enabled = view.glow_draws();
    let settings = &mut view.atmosphere;
    use crate::widgets::{choice_row, ValueBar};

    super::section(ui, "Glow pattern", |ui| {
        if !glow_enabled {
            crate::widgets::label(
                ui,
                "Set Background glow reach and gain above zero to see these effects.",
            );
        }
        ui.add_enabled_ui(glow_enabled, |ui| {
            ValueBar::new(&mut settings.texture_depth, 0.0..=1.0, "Pattern contrast")
                .default_value(super::fresh_view().atmosphere.texture_depth)
                .percent().show(ui).on_hover_text("Pattern contrast before the material shapes the light. 0% restores smooth halos; material settings remain active.");
            ValueBar::new(&mut settings.texture_scale, NEBULA_SCALE_MIN..=NEBULA_SCALE_MAX, "Pattern size")
                .default_value(super::fresh_view().atmosphere.texture_scale)
                .eased(true)
                .unit(1.0, "×")
                .show(ui)
                .on_hover_text("Larger values make broader patterns. The material can reshape this pattern.");
            multiplier(ui, (&mut settings.texture_speed, super::fresh_view().atmosphere.texture_speed), "Pattern speed", NEBULA_SPEED_MIN..=NEBULA_SPEED_MAX)
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
            crate::widgets::label(
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
                .default_value(super::fresh_view().atmosphere.material_amount)
                .percent().show(ui).on_hover_text("How strongly the material reshapes the textured light. 0% bypasses the material while preserving the texture.");
            if settings.material_style == LatticeMaterial::Stars {
                super::material::stars_resolution(ui, &mut settings.stars, &super::fresh_view().atmosphere.stars);
            }
            // Stars put their look first: size and spacing matter more than
            // how they drift.
            let stars = settings.material_style == LatticeMaterial::Stars;
            if stars {
                super::block(ui, "Appearance", |ui| {
                    super::material::stars(ui, &mut settings.stars, &super::fresh_view().atmosphere.stars, harmonigraph_scene::StarPane::Lattice);
                });
            }
            super::block(ui, "Motion", |ui| {
                if stars {
                    super::material::stars_motion(ui, &mut settings.stars, &super::fresh_view().atmosphere.stars);
                }
                let speed = (!stars).then_some(&mut settings.material_speed);
                let fresh = &super::fresh_view().atmosphere;
                crate::widgets::drift(
                    ui,
                    &mut settings.material_direction,
                    speed,
                    (fresh.material_direction, fresh.material_speed),
                );
            });
            match settings.material_style {
                LatticeMaterial::Watercolor => {
                    super::block(ui, "Appearance", |ui| {
                        super::material::watercolor(ui, &mut settings.material_settings, &super::fresh_view().atmosphere.material_settings);
                    });
                }
                LatticeMaterial::VelvetScales => {
                    super::block(ui, "Appearance", |ui| {
                        super::material::velvet(ui, &mut settings.material_settings, &super::fresh_view().atmosphere.material_settings);
                    });
                }
                LatticeMaterial::Stars | LatticeMaterial::None => {},
            }
        }
        crate::widgets::button_row(ui, |ui| {
            if ui.button("Reset material").on_hover_text("Reset only the material choice, amount, motion and geometry controls.").clicked() {
                let fresh = AtmosphereSettings::default();
                settings.material_style = fresh.material_style;
                settings.material_amount = fresh.material_amount;
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
    (value, fresh): (&mut f32, f32),
    label: &str,
    range: std::ops::RangeInclusive<f32>,
) -> egui::Response {
    crate::widgets::ValueBar::new(value, range, label).unit(1.0, "×").default_value(fresh).show(ui)
}
