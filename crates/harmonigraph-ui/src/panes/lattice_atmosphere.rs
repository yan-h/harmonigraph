//! Controls for the lattice's nebula glow and breathing.

use harmonigraph_scene::{
    AtmosphereSettings, BREATH_SPEED_MAX, BREATH_SPEED_MIN, NEBULA_SCALE_MAX, NEBULA_SCALE_MIN,
    NEBULA_SPEED_MAX, NEBULA_SPEED_MIN,
};

/// A block of the Light section whose name is its switch: the texture and the
/// breathing are one effect on the glow, and off, the rows under it would set
/// nothing that draws. Glow itself off, the switch and its rows grey instead:
/// they keep what they are set to for when the glow comes back.
pub(super) fn settings(ui: &mut egui::Ui, view: &mut harmonigraph_scene::ViewConfig) {
    let glow_enabled = view.glow_reach > 0.0 && view.glow_strength > 0.0;
    let settings = &mut view.atmosphere;
    use crate::widgets::ValueBar;

    crate::widgets::group_space(ui);
    ui.add_enabled_ui(glow_enabled, |ui| {
        crate::widgets::checkbox(
            ui,
            &mut settings.enabled,
            egui::RichText::new("Glow texture and breathing").strong(),
        )
        .on_hover_text(
            "Cloud texture and slow breathing in the lattice glow. \
             Turning this off preserves both effects' settings.",
        );
    });
    if !glow_enabled {
        crate::widgets::weak(ui, "Set Glow reach and Glow gain above zero to see these effects.");
    }
    if !settings.enabled {
        return;
    }
    ui.add_enabled_ui(glow_enabled, |ui| {
    ValueBar::new(&mut settings.nebula_depth, 0.0..=1.0, "Texture depth")
        .percent().show(ui).on_hover_text("Cloud texture in the combined lattice glow. 0% restores smooth halos. Colors come from the notes.");
    multiplier(ui, &mut settings.nebula_scale, "Cloud size", NEBULA_SCALE_MIN..=NEBULA_SCALE_MAX)
        .on_hover_text("Size of the cloud texture relative to the pane. Larger values make broader clouds; lattice zoom does not resize the texture. 1× is the reference size.");
    multiplier(ui, &mut settings.nebula_speed, "Cloud speed", NEBULA_SPEED_MIN..=NEBULA_SPEED_MAX)
        .on_hover_text("1× is a slow drift. 0 freezes the cloud motion.");
    ValueBar::new(&mut settings.breath_amount, 0.0..=1.0, "Breathing depth")
        .percent().show(ui).on_hover_text("Brightness variation in the existing lattice glow. 0% keeps it steady; 100% allows deep fades. Does not change note brightness directly.");
    multiplier(ui, &mut settings.breath_speed, "Breathing speed", BREATH_SPEED_MIN..=BREATH_SPEED_MAX)
        .on_hover_text("1× is the original slow breathing. 0 keeps the halo at its normal brightness.");
    });
    crate::widgets::button_row(ui, |ui| {
        if ui
            .button("Reset effects")
            .on_hover_text("Reset only the texture and breathing settings above.")
            .clicked()
        {
            *settings = AtmosphereSettings::default();
        }
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
