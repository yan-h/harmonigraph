//! An opening threshold and an independently stored hysteresis band.
use super::plot::{number, Plot};
use crate::theme;
use egui::Ui;

pub(crate) fn threshold(ui: &mut Ui, gate: &mut f32, hysteresis: &mut f32) {
    use harmonigraph_scene::SPECTRAL_HYSTERESIS_MAX;
    let plot = Plot::new(ui, "Ring threshold · on / off");
    // Include the full negative extent of the stored band. Clipping it at zero
    // would silently discard hysteresis when the opening threshold is lowered.
    let x = |v: f32| (v + SPECTRAL_HYSTERESIS_MAX) / (1.0 + SPECTRAL_HYSTERESIS_MAX);
    let value = |x: f32| x * (1.0 + SPECTRAL_HYSTERESIS_MAX) - SPECTRAL_HYSTERESIS_MAX;
    let (_, next) = plot.handle(ui, "Turn on", x(*gate), 0.8);
    if let Some(p) = next {
        *gate = value(p.x).clamp(0.0, 1.0);
    }
    let (_, next) = plot.handle(ui, "Hysteresis below turn-on", x(*gate - *hysteresis), 0.2);
    if let Some(p) = next {
        *hysteresis = (*gate - value(p.x)).clamp(0.0, SPECTRAL_HYSTERESIS_MAX);
    }
    number(ui, gate, 0.0..=1.0, "Ring threshold", 100.0, "%");
    number(ui, hysteresis, 0.0..=SPECTRAL_HYSTERESIS_MAX, "Threshold hysteresis", 100.0, " pp");
    ui.painter().rect_filled(
        egui::Rect::from_two_pos(
            plot.point(x(*gate - *hysteresis), 0.2),
            plot.point(x(*gate), 0.8),
        ),
        0,
        theme::accent_fill(),
    );
    plot.line(
        ui,
        vec![plot.point(x(*gate - *hysteresis), 0.2), plot.point(x(*gate), 0.8)],
        theme::accent(),
    );
    plot.line(ui, vec![plot.point(x(0.0), 0.0), plot.point(x(0.0), 1.0)], theme::hairline());
    // The level buffer has 255 steps. Match its anti-latching floor, including
    // the distinct gate=0 state which deliberately shows silent rings.
    let off = (*gate - *hysteresis).max(gate.min(1.0 / 255.0));
    plot.line(ui, vec![plot.point(x(off), 0.0), plot.point(x(off), 0.45)], theme::text());
    super::weak(
        ui,
        if *gate == 0.0 {
            "All rings, including silence".to_owned()
        } else {
            format!("On {:.1}% · off below {:.2}%", *gate * 100.0, off * 100.0)
        },
    );
    plot.dot(ui, x(*gate), 0.8);
    plot.dot(ui, x(*gate - *hysteresis), 0.2);
}
