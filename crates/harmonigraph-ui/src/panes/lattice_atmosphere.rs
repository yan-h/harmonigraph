//! Independent texture and material stages in the lattice's background glow.

use harmonigraph_scene::{
    AtmosphereSettings, LatticeMaterial, LatticeTexture, NEBULA_SCALE_MAX, NEBULA_SCALE_MIN,
    NEBULA_SPEED_MAX, NEBULA_SPEED_MIN, SHADOW_PICKUP_SIZE_MAX,
};

pub(super) fn settings(ui: &mut egui::Ui, view: &mut harmonigraph_scene::ViewConfig) {
    let glow_enabled = view.glow_reach > 0.0 && view.glow_strength > 0.0;
    let settings = &mut view.atmosphere;
    use crate::widgets::{choice_row, ValueBar};

    super::section(ui, "Glow pattern", |ui| {
        crate::widgets::weak(ui, "Pattern varies the glow; material reshapes the result.");
        if !glow_enabled {
            crate::widgets::weak(
                ui,
                "Set Background glow reach and gain above zero to see these effects.",
            );
        }
        ui.add_enabled_ui(glow_enabled, |ui| {
        choice_row(ui, "Pattern", &mut settings.texture, &[
            (LatticeTexture::None, "None", "Smooth note light; keeps the pattern settings"),
            (LatticeTexture::Clouds, "Clouds", "Softly drifting clouds in the note light"),
            (LatticeTexture::Contours, "Contours", "Nested bands following the combined note light"),
            (LatticeTexture::Interference, "Interference", "Curved wave fringes illuminated by the notes"),
        ]);
        if settings.texture != LatticeTexture::None {
            ValueBar::new(&mut settings.texture_depth, 0.0..=1.0, "Pattern contrast")
                .percent().show(ui).on_hover_text("Pattern contrast before the material shapes the light. 0% restores smooth halos; material settings remain active.");
            multiplier(ui, &mut settings.texture_scale, "Pattern size", NEBULA_SCALE_MIN..=NEBULA_SCALE_MAX)
                .on_hover_text("Larger values make broader patterns and fewer contour bands. The material can reshape this pattern.");
            multiplier(ui, &mut settings.texture_speed, "Pattern speed", NEBULA_SPEED_MIN..=NEBULA_SPEED_MAX)
                .on_hover_text("Pattern motion before the material. 0 freezes this motion; notes and the material can still change the picture.");
        }
        crate::widgets::button_row(ui, |ui| {
            if ui.button("Reset pattern").on_hover_text("Reset only the pattern choice, contrast, size and speed.").clicked() {
                let fresh = AtmosphereSettings::default();
                settings.texture = fresh.texture;
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
            (LatticeMaterial::VelvetScales, "Velvet Scales", "Soft overlapping scallops carrying the note light"),
            (LatticeMaterial::Mosaic, "Mosaic", "Soft-edged facets of the textured note light"),
            (LatticeMaterial::Stars, "Stars", "Drifting stars colored by the note light"),
        ]);
        if settings.material_style != LatticeMaterial::None {
            ValueBar::new(&mut settings.material_amount, 0.0..=1.0, "Material amount")
                .percent().show(ui).on_hover_text("How strongly the material reshapes the textured light. 0% bypasses the material while preserving the texture.");
            ValueBar::new(&mut settings.material_shadow_pickup, 0.0..=1.0, "Dark pickup")
                .percent().show(ui).on_hover_text("Dark pigment behind unlit ring segments, picked up by the material. Fades as each segment lights up. Actual ring and label shadows keep their own settings.");
            ValueBar::new(&mut settings.material_color_pickup, 0.0..=1.0, "Color pickup")
                .percent().show(ui).on_hover_text("Pitch-colored pigment behind lit ring segments, picked up by the material. Follows each segment’s activation and Bloom brightness. Spreads by distance around the segment, including its ends. 0% disables colored pickup.");
            ValueBar::new(&mut settings.material_shadow_width, 0.0..=SHADOW_PICKUP_SIZE_MAX, "Pickup width")
                .percent().show(ui).on_hover_text("Full width of the pigment source band, as a percentage of the node radius. Up to 800% for broad washes. 0% disables pickup; actual shadow width is unchanged.");
            ValueBar::new(&mut settings.material_shadow_softness, 0.0..=SHADOW_PICKUP_SIZE_MAX, "Pickup softness")
                .percent().show(ui).on_hover_text("Soft fade by distance around each segment, including its rounded ends, as a percentage of the node radius. Up to 800% for very diffuse pigment. Does not alter the actual shadow.");
            super::block(ui, "Motion");
            if settings.material_style == LatticeMaterial::Stars {
                super::material::stars_motion(ui, &mut settings.stars);
            }
            let speed = (settings.material_style != LatticeMaterial::Stars).then_some(&mut settings.material_speed);
            crate::widgets::drift(ui, &mut settings.material_direction, speed);
            super::block(ui, "Appearance");
            match settings.material_style {
                LatticeMaterial::Watercolor => super::material::watercolor(ui, &mut settings.material_settings),
                LatticeMaterial::VelvetScales => super::material::velvet(ui, &mut settings.material_settings),
                LatticeMaterial::Mosaic => super::material::mosaic(ui, &mut settings.material_settings),
                LatticeMaterial::Stars => super::material::stars(ui, &mut settings.stars),
                LatticeMaterial::None => {},
            }
        }
        crate::widgets::button_row(ui, |ui| {
            if ui.button("Reset material").on_hover_text("Reset only the material choice, amount, motion and geometry controls.").clicked() {
                let fresh = AtmosphereSettings::default();
                settings.material_style = fresh.material_style;
                settings.material_amount = fresh.material_amount;
                settings.material_shadow_pickup = fresh.material_shadow_pickup;
                settings.material_color_pickup = fresh.material_color_pickup;
                settings.material_shadow_width = fresh.material_shadow_width;
                settings.material_shadow_softness = fresh.material_shadow_softness;
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
