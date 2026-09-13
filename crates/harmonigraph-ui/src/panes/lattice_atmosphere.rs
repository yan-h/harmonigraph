//! Controls for the lattice's nebula glow, wide halo and breathing.

use harmonigraph_scene::AtmosphereSettings;

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
        multiplier(ui, &mut settings.nebula_scale, "Cloud size", 0.25..=4.0);
        multiplier(ui, &mut settings.nebula_speed, "Cloud speed", 0.0..=20.0)
            .on_hover_text("1× is a slow drift. 0 freezes the cloud motion.");

        ui.label(egui::RichText::new("Wide glow").strong());
        ValueBar::new(&mut settings.wide_strength, 0.0..=1.0, "Amount")
            .percent().show(ui).on_hover_text("A faint second halo with the note's average glow color. It lights the lattice too. 0 restores the close halo alone; requires Lattice glow below.");
        multiplier(ui, &mut settings.wide_spread, "Spread", 1.0..=6.0)
            .on_hover_text("Outer radius relative to the close halo. The close halo keeps its existing range and falloff.");

        ui.label(egui::RichText::new("Breathing halos").strong());
        ValueBar::new(&mut settings.breath_amount, 0.0..=1.0, "Breathing depth")
            .percent().show(ui).on_hover_text("Brightness variation in the existing lattice glow. 0% keeps it steady; 100% allows deep fades. Requires Lattice glow to be enabled below.");
        multiplier(ui, &mut settings.breath_speed, "Breathing speed", 0.0..=4.0)
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
