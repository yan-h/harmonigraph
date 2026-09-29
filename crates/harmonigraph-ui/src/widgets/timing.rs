//! Time-domain editors: note envelope and independent rise/fall time constants.
use super::plot::{curve, number, Plot};
use crate::theme;
use egui::{Response, Ui};

pub(crate) fn fade(ui: &mut Ui, seconds: &mut f32, shape: &mut f32) -> Response {
    let plot = Plot::new(ui, "Note fade · duration and curve");
    let (duration_response, next) = plot.handle(ui, "Fade duration", *seconds, 1.0);
    if let Some(p) = next {
        *seconds = p.x;
    }
    let sample = |shape, p| {
        harmonigraph_core::Envelope { attack_time: 1.0, shape, ..Default::default() }.attack(p, 0.0)
    };
    let (_, next) = plot.handle(ui, "Fade shape", *seconds * 0.5, sample(*shape, 0.5));
    if let Some(p) = next {
        // Invert the production envelope at its midpoint, retaining its single shape dial.
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..24 {
            let mid = (lo + hi) * 0.5;
            if sample(mid, 0.5) < p.y {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        *shape = (lo + hi) * 0.5;
    }
    let numeric = number(ui, seconds, 0.0..=1.0, "Note fade", 1000.0, " ms");
    number(ui, shape, 0.0..=1.0, "Fade curve", 100.0, "%");
    curve(&plot, ui, |p| (p * *seconds, sample(*shape, p as f64)), super::value::curve_color());
    plot.dot(ui, *seconds, 1.0);
    plot.dot(ui, *seconds * 0.5, sample(*shape, 0.5));
    duration_response | numeric
}

pub(crate) fn response(
    ui: &mut Ui,
    rise: &mut f32,
    fall: &mut f32,
    max: f32,
    labels: [&str; 2],
    unit: f32,
) {
    ui.push_id(labels[0], |ui| {
        let plot = Plot::new(ui, "Response · rise / fall");
        let rise_y = |t:f32| if t == 0.0 { 1.0 } else { 1.0 - (-1.0f32).exp() };
        let fall_y = |t:f32| if t == 0.0 { 0.0 } else { (-1.0f32).exp() };
        let (_,next)=plot.handle(ui,labels[0],0.45*(*rise/max).sqrt(),rise_y(*rise));
        if let Some(p)=next { *rise=(p.x/0.45).min(1.0).powi(2)*max; }
        let (_,next)=plot.handle(ui,labels[1],0.55+0.45*(*fall/max).sqrt(),fall_y(*fall));
        if let Some(p)=next { *fall=((p.x-0.55)/0.45).clamp(0.0,1.0).powi(2)*max; }
        let suffix=if unit==1000.0 {" ms"}else{" s"};
        number(ui,rise,0.0..=max,labels[0],unit,suffix);
        number(ui,fall,0.0..=max,labels[1],unit,suffix);
        curve(&plot,ui,|p| {let t=p*p*max; (0.45*p,if *rise==0.0 {1.0}else{1.0-(-t / *rise).exp()})},theme::accent());
        curve(&plot,ui,|p| {let t=p*p*max; (0.55+0.45*p,if *fall==0.0 {0.0}else{(-t / *fall).exp()})},theme::accent());
        plot.dot(ui,0.45*(*rise/max).sqrt(),rise_y(*rise));
        plot.dot(ui,0.55+0.45*(*fall/max).sqrt(),fall_y(*fall));
        plot.response.on_hover_text("Independent exponential response times. Each handle marks one time constant: 63% risen or 37% remaining. Zero responds immediately. The time axis is expanded near zero.");
    });
}
