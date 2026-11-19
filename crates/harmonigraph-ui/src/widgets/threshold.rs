//! An opening threshold and an independently stored hysteresis band.
use super::plot::{value_bar, Plot};
use crate::theme;
use egui::Ui;

/// `fresh` is where a double-click puts the gate and the band back.
pub(crate) fn threshold(
    ui: &mut Ui,
    gate: &mut f32,
    hysteresis: &mut f32,
    fresh: (f32, f32),
) -> egui::Response {
    use harmonigraph_scene::{
        closing_threshold, SPECTRAL_GATE_MAX, SPECTRAL_GATE_MIN, SPECTRAL_HYSTERESIS_MAX,
    };
    let plot = Plot::with_fields(ui, "Ring threshold", 3);
    // Include the full negative extent of the stored band. Clipping it at the
    // gate's floor would silently discard hysteresis when the opening threshold
    // is lowered.
    let (low, high) = (SPECTRAL_GATE_MIN - SPECTRAL_HYSTERESIS_MAX, SPECTRAL_GATE_MAX);
    let x = |v: f32| (v - low) / (high - low);
    let value = |x: f32| low + x * (high - low);
    let (_, next) = plot.handle(ui, "Turn on", x(*gate), 0.8);
    if let Some(p) = next {
        *gate = value(p.x).clamp(SPECTRAL_GATE_MIN, SPECTRAL_GATE_MAX);
    }
    let (_, next) = plot.handle(ui, "Hysteresis below turn-on", x(*gate - *hysteresis), 0.2);
    if let Some(p) = next {
        *hysteresis = (*gate - value(p.x)).clamp(0.0, SPECTRAL_HYSTERESIS_MAX);
    }
    plot.fields(ui, |ui| {
        value_bar(
            ui,
            (gate, fresh.0),
            SPECTRAL_GATE_MIN..=SPECTRAL_GATE_MAX,
            ["Ring threshold", "On"],
            100.0,
            "%",
        );
        value_bar(
            ui,
            (hysteresis, fresh.1),
            0.0..=SPECTRAL_HYSTERESIS_MAX,
            ["Threshold hysteresis", "Band"],
            100.0,
            " pp",
        );
        let off = closing_threshold(*gate, *hysteresis);
        let status = if *gate == SPECTRAL_GATE_MIN {
            "Includes silence".to_owned()
        } else {
            format!("Off < {:.2}%", off * 100.0)
        };
        ui.add(egui::Label::new(egui::RichText::new(status.clone())).truncate())
            .on_hover_text(status);
    });
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
    let floor = x(SPECTRAL_GATE_MIN);
    plot.line(ui, vec![plot.point(floor, 0.0), plot.point(floor, 1.0)], theme::hairline());
    // Where the scene's gate lets an open bucket go, its anti-latching floor
    // and the gate's own floor, which deliberately shows silent rings, included.
    let off = closing_threshold(*gate, *hysteresis);
    plot.line(ui, vec![plot.point(x(off), 0.0), plot.point(x(off), 0.45)], theme::text());
    plot.dot(ui, x(*gate), 0.8);
    plot.dot(ui, x(*gate - *hysteresis), 0.2);
    plot.response
}
