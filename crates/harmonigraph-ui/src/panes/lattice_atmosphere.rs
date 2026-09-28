//! Independent texture and material stages in the lattice's background glow.

use harmonigraph_scene::{
    AtmosphereSettings, LatticeMaterial, LatticeTexture, NEBULA_SCALE_MAX, NEBULA_SCALE_MIN,
    NEBULA_SPEED_MAX, NEBULA_SPEED_MIN,
};

pub(super) fn settings(ui: &mut egui::Ui, view: &mut harmonigraph_scene::ViewConfig) {
    let glow_enabled = view.glow_reach > 0.0 && view.glow_strength > 0.0;
    let settings = &mut view.atmosphere;
    use crate::widgets::{choice_row, ValueBar};

    super::block(ui, "Background glow texture");
    if !glow_enabled {
        crate::widgets::weak(
            ui,
            "Set Background glow reach and gain above zero to see these effects.",
        );
    }
    ui.add_enabled_ui(glow_enabled, |ui| {
        choice_row(ui, "Texture", &mut settings.texture, &[
            (LatticeTexture::None, "None", "Smooth note light; keeps the texture settings"),
            (LatticeTexture::Clouds, "Clouds", "Softly drifting clouds in the note light"),
            (LatticeTexture::Contours, "Contours", "Nested bands following the combined note light"),
            (LatticeTexture::Interference, "Interference", "Curved wave fringes illuminated by the notes"),
        ]);
        if settings.texture != LatticeTexture::None {
            ValueBar::new(&mut settings.texture_depth, 0.0..=1.0, "Texture depth")
                .percent().show(ui).on_hover_text("Pattern contrast before the material shapes the light. 0% restores smooth halos; material settings remain active.");
            multiplier(ui, &mut settings.texture_scale, "Texture size", NEBULA_SCALE_MIN..=NEBULA_SCALE_MAX)
                .on_hover_text("Larger values make broader patterns and fewer contour bands. The material can reshape this pattern.");
            multiplier(ui, &mut settings.texture_speed, "Texture speed", NEBULA_SPEED_MIN..=NEBULA_SPEED_MAX)
                .on_hover_text("Texture motion before the material. 0 freezes this motion; notes and the material can still change the picture.");
        }
        crate::widgets::button_row(ui, |ui| {
            if ui.button("Reset texture").on_hover_text("Reset only the texture choice, depth, size and speed.").clicked() {
                let fresh = AtmosphereSettings::default();
                settings.texture = fresh.texture;
                settings.texture_depth = fresh.texture_depth;
                settings.texture_scale = fresh.texture_scale;
                settings.texture_speed = fresh.texture_speed;
            }
        });
    });
    super::block(ui, "Background glow material");
    ui.add_enabled_ui(glow_enabled, |ui| {
        choice_row(ui, "Material", &mut settings.material_style, &[
            (LatticeMaterial::None, "None", "Keep the textured light as it is; preserves material settings"),
            (LatticeMaterial::Watercolor, "Watercolor", "Overlapping washes of the textured note light"),
            (LatticeMaterial::Mosaic, "Mosaic", "Soft-edged facets of the textured note light"),
        ]);
        if settings.material_style != LatticeMaterial::None {
            ValueBar::new(&mut settings.material_amount, 0.0..=1.0, "Material amount")
                .percent().show(ui).on_hover_text("How strongly the material reshapes the textured light. 0% bypasses the material while preserving the texture.");
            ValueBar::new(&mut settings.material_shadow_pickup, 0.0..=1.0, "Shadow pickup")
                .percent().show(ui).on_hover_text("How much node-shadow darkness the washes or facets pick up. 0% keeps smooth shadows over the material; 100% lets the material carry them. Ring and label occlusion stay local.");
            super::material::speed(ui, &mut settings.material_speed);
            super::material::direction(ui, &mut settings.material_direction);
            match settings.material_style {
                LatticeMaterial::Watercolor => super::material::watercolor(ui, &mut settings.material_settings),
                LatticeMaterial::Mosaic => super::material::mosaic(ui, &mut settings.material_settings),
                LatticeMaterial::None => {},
            }
        }
        crate::widgets::button_row(ui, |ui| {
            if ui.button("Reset material").on_hover_text("Reset only the material choice, amount, motion and geometry controls.").clicked() {
                let fresh = AtmosphereSettings::default();
                settings.material_style = fresh.material_style;
                settings.material_amount = fresh.material_amount;
                settings.material_shadow_pickup = fresh.material_shadow_pickup;
                settings.material_settings = fresh.material_settings;
                settings.material_speed = fresh.material_speed;
                settings.material_direction = fresh.material_direction;
            }
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
