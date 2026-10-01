//! Direction and length are edited as a vector, beside exact numeric entry.
use super::plot::{value_bar, Plot};
use crate::theme;
use egui::Ui;

pub(crate) fn drift(ui: &mut Ui, degrees: &mut f32, mut speed: Option<&mut f32>) {
    ui.push_id("drift", |ui| {
        let plot=Plot::square_with_fields(ui, "Drift", if speed.is_some() { 2 } else { 1 });
        let max=harmonigraph_scene::CLOUD_SPEED_MAX;
        let has_speed=speed.is_some();
        // Square-root radius keeps slow motion selectable beside the 20× ceiling.
        let radius=speed.as_deref().map_or(1.0, |s| (*s/max).sqrt());
        let angle=degrees.to_radians();
        let x=0.5+0.5*radius*angle.cos();
        let y=0.5-0.5*radius*angle.sin();
        let (_, next)=plot.handle(ui,"Direction and speed",x,y);
        if let Some(p)=next {
            let v=egui::vec2((p.x-0.5)/0.5,(0.5-p.y)/0.5);
            if v.length()>0.001 { *degrees=v.y.atan2(v.x).to_degrees().rem_euclid(360.0); }
            if let Some(s)=speed.as_deref_mut() { *s=v.length().min(1.0).powi(2)*max; }
        }
        plot.fields(ui, |ui| {
        if value_bar(ui,degrees,0.0..=360.0,["Drift direction", "Direction"],1.0,"°").changed() {
            *degrees = degrees.rem_euclid(360.0);
        }
        if let Some(s)=speed.as_deref_mut() { value_bar(ui,s,0.0..=max,["Drift speed", "Speed"],1.0,"×"); }
        });
        let radius=speed.as_deref().map_or(1.0, |s| (*s/max).sqrt());
        let a=degrees.to_radians();
        let (x,y)=(0.5+0.5*radius*a.cos(),0.5-0.5*radius*a.sin());
        plot.line(ui,vec![plot.point(0.0,0.5),plot.point(1.0,0.5)],theme::hairline());
        plot.line(ui,vec![plot.point(0.5,0.0),plot.point(0.5,1.0)],theme::hairline());
        let start = plot.point(0.5,0.5);
        let end = plot.point(x,y);
        if start.distance(end)>0.01 {
            ui.painter().arrow(start,end-start,egui::Stroke::new(1.5,theme::accent()));
        }
        plot.dot(ui,x,y);
        // Stars set their own pace under Star speed and pass no speed here.
        plot.response.on_hover_text(if has_speed {
            "Right 0° · Down 90° · Left 180° · Up 270°. Arrow keys adjust the handle; Shift gives finer steps. 1× moves the material about a pane-height every four minutes."
        } else {
            "Right 0° · Down 90° · Left 180° · Up 270°. Arrow keys adjust the handle; Shift gives finer steps."
        });
    });
}

pub(crate) fn cabinet(ui: &mut Ui, angle: &mut f32, length: &mut f32) {
    ui.push_id("cabinet", |ui| {
        let plot = Plot::square_with_fields(ui, "Depth axis", 2);
        let (_, next) = plot.handle(ui, "Depth axis", *length * angle.cos(), *length * angle.sin());
        if let Some(p) = next {
            if p.length() > 0.001 {
                *angle = p.y.atan2(p.x);
            }
            *length = p.length().clamp(0.1, 1.0);
        }
        plot.fields(ui, |ui| {
            let mut deg = angle.to_degrees();
            if value_bar(ui, &mut deg, 0.0..=90.0, ["Depth angle", "Angle"], 1.0, "°").changed() {
                *angle = deg.to_radians();
            }
            value_bar(ui, length, 0.1..=1.0, ["Depth step scale", "Length"], 1.0, "×");
        });
        plot.line(
            ui,
            vec![plot.point(0.0, 1.0), plot.point(0.0, 0.0), plot.point(1.0, 0.0)],
            theme::hairline(),
        );
        plot.line(
            ui,
            vec![plot.point(0.0, 0.0), plot.point(*length * angle.cos(), *length * angle.sin())],
            theme::accent(),
        );
        plot.dot(ui, *length * angle.cos(), *length * angle.sin());
    });
}

pub(crate) fn softness(ui: &mut Ui, pitch: &mut f32, time: &mut f32) {
    use harmonigraph_scene::{PITCH_SOFTNESS_MAX, TIME_SOFTNESS_MAX};
    ui.push_id("softness", |ui| {
        let plot = Plot::with_fields(ui, "Softness · time → / pitch ↑", 2);
        let (_, next) = plot.handle(
            ui,
            "Time and pitch softness",
            (*time / TIME_SOFTNESS_MAX).sqrt(),
            (*pitch / PITCH_SOFTNESS_MAX).sqrt(),
        );
        if let Some(p) = next {
            *time = p.x * p.x * TIME_SOFTNESS_MAX;
            *pitch = p.y * p.y * PITCH_SOFTNESS_MAX;
        }
        plot.fields(ui, |ui| {
            value_bar(ui, pitch, 0.0..=PITCH_SOFTNESS_MAX, ["Pitch softness", "Pitch"], 1.0, "¢");
            value_bar(ui, time, 0.0..=TIME_SOFTNESS_MAX, ["Time softness", "Time"], 1.0, " ms");
        });
        let w = (*time / TIME_SOFTNESS_MAX).sqrt();
        let h = (*pitch / PITCH_SOFTNESS_MAX).sqrt();
        // The handle is the footprint's bounding corner. Guides make both
        // independently selected widths explicit, including a zero-width blur.
        plot.line(
            ui,
            vec![plot.point(0.0, h), plot.point(w, h), plot.point(w, 0.0)],
            theme::hairline(),
        );
        plot.line(
            ui,
            (0..=64)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::TAU / 64.0;
                    plot.point(w * 0.5 * (1.0 + a.cos()), h * 0.5 * (1.0 + a.sin()))
                })
                .collect(),
            theme::accent(),
        );
        plot.dot(ui, w, h);
    });
}
