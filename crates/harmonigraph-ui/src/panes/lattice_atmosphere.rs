//! Controls for the lattice's nebula glow, dusk wash and breathing.

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

        ui.label(egui::RichText::new("Dusk wash").strong());
        multiplier(ui, &mut settings.dusk_strength, "Color depth", 0.0..=3.0)
            .on_hover_text("Broad pools of dusk color behind the lattice. 0 removes the wash.");
        ValueBar::new(&mut settings.dusk_warmth, 0.0..=1.0, "Warmth")
            .percent().show(ui);
        multiplier(ui, &mut settings.dusk_speed, "Drift speed", 0.0..=4.0)
            .on_hover_text("0 freezes the pools; glow color can still influence their tint.");
        ValueBar::new(&mut settings.dusk_response, 0.0..=1.0, "Follow glow color")
            .percent().show(ui).on_hover_text("Gently borrows the average foreground glow color, including its existing attack and release fades. 0 keeps the dusk palette independent.");

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
