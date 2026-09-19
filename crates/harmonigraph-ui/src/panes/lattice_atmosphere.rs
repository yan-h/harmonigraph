//! Controls for the lattice's nebula glow and breathing.

use harmonigraph_scene::{
    AtmosphereSettings, BREATH_SPEED_MAX, BREATH_SPEED_MIN, NEBULA_SCALE_MAX, NEBULA_SCALE_MIN,
    NEBULA_SPEED_MAX, NEBULA_SPEED_MIN,
};

pub(super) fn settings(ui: &mut egui::Ui, settings: &mut AtmosphereSettings) {
    use crate::widgets::{toggle_switch, ValueBar};

    super::section(ui, "Atmosphere (prototype)");
    toggle_switch(ui, &mut settings.enabled, "Enabled");
    if ui.small_button("Reset defaults").clicked() {
        *settings = AtmosphereSettings::default();
    }
    ui.add_enabled_ui(settings.enabled, |ui| {
        ui.label(egui::RichText::new("Nebula glow").strong());
        ValueBar::new(&mut settings.nebula_depth, 0.0..=1.0, "Texture depth")
            .percent().show(ui).on_hover_text("Cloud texture in the combined lattice glow. 0% restores smooth halos. Requires Lattice glow below; colors come from the notes.");
        multiplier(ui, &mut settings.nebula_scale, "Cloud size", NEBULA_SCALE_MIN..=NEBULA_SCALE_MAX);
        multiplier(ui, &mut settings.nebula_speed, "Cloud speed", NEBULA_SPEED_MIN..=NEBULA_SPEED_MAX)
            .on_hover_text("1× is a slow drift. 0 freezes the cloud motion.");

        ui.label(egui::RichText::new("Breathing halos").strong());
        ValueBar::new(&mut settings.breath_amount, 0.0..=1.0, "Breathing depth")
            .percent().show(ui).on_hover_text("Brightness variation in the existing lattice glow. 0% keeps it steady; 100% allows deep fades. Requires Lattice glow to be enabled below.");
        multiplier(ui, &mut settings.breath_speed, "Breathing speed", BREATH_SPEED_MIN..=BREATH_SPEED_MAX)
            .on_hover_text("1× is the original slow breathing. 0 keeps the halo at its normal brightness.");
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
