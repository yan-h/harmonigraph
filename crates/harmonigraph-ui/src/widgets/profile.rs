//! Spatial profiles edited through their endpoints and curve, with numeric fallbacks.
use super::plot::{curve, number, Plot};
use crate::theme;
use egui::Ui;
use std::ops::RangeInclusive;

pub(crate) fn depth(
    ui: &mut Ui,
    low: &mut f32,
    high: &mut f32,
    exponent: &mut f32,
    range: RangeInclusive<f32>,
    curve_range: RangeInclusive<f32>,
    size: bool,
) {
    let (name, curve_name, unit, suffix) = if size {
        ("Star size", "Size curve", 1.0, " px")
    } else {
        ("Star speed", "Speed curve", 100.0, "%")
    };
    ui.push_id(name, |ui| {
        let plot = Plot::new(ui, &format!("{name} · far → near"));
        let encode = |v: f32| {
            if size {
                (v.log2() - range.start().log2()) / (range.end().log2() - range.start().log2())
            } else {
                (v - range.start()) / (range.end() - range.start())
            }
        };
        let decode = |p: f32| {
            if size {
                (range.start().log2() + p * (range.end().log2() - range.start().log2())).exp2()
            } else {
                range.start() + p * (range.end() - range.start())
            }
        };
        let (_, next) = plot.handle(ui, "Far depth", 0.0, encode(*low));
        if let Some(p) = next {
            *low = decode(p.y).min(*high);
        }
        let (_, next) = plot.handle(ui, "Near depth", 1.0, encode(*high));
        if let Some(p) = next {
            *high = decode(p.y).max(*low);
        }
        // The shape is a power of depth. Star sizes interpolate in log space;
        // speed interpolates linearly. Those are also the plotted units.
        let a = encode(*low);
        let b = encode(*high);
        let (_, next) =
            plot.handle(ui, "Depth distribution", 0.5, a + (b - a) * 0.5f32.powf(*exponent));
        if let Some(p) = next {
            if b - a > 1e-6 {
                *exponent = (((p.y - a) / (b - a)).clamp(0.0001, 0.9999).ln() / 0.5f32.ln())
                    .clamp(*curve_range.start(), *curve_range.end());
            }
        }
        // No log round-trip on idle frames: values also key star placement.
        number(ui, low, *range.start()..=*high, &format!("{name} far"), unit, suffix);
        number(ui, high, *low..=*range.end(), &format!("{name} near"), unit, suffix);
        number(ui, exponent, curve_range, curve_name, 1.0, "");
        let a = encode(*low);
        let b = encode(*high);
        curve(&plot, ui, |p| (p, a + (b - a) * p.powf(*exponent)), theme::accent());
        plot.dot(ui, 0.0, a);
        plot.dot(ui, 1.0, b);
        plot.dot(ui, 0.5, a + (b - a) * 0.5f32.powf(*exponent));
    });
}

pub(crate) fn shadow(
    ui: &mut Ui,
    style: &mut harmonigraph_scene::ShadowStyle,
    max: f32,
    lattice: bool,
) {
    use harmonigraph_scene::{SHADOW_FALLOFF_MAX, SHADOW_FALLOFF_MIN};
    let plot = Plot::new(ui, "Shadow profile · width / darkness / shape");
    let (_, next) = plot.handle(ui, "Width and darkness", (style.width / max).sqrt(), style.depth);
    if let Some(p) = next {
        style.width = p.x * p.x * max;
        style.depth = p.y;
    }
    if style.kernel.is_distance() {
        let level = harmonigraph_scene::standoff_level(style.falloff, 0.5);
        let (_, next) =
            plot.handle(ui, "Falloff", 0.5 * (style.width / max).sqrt(), style.depth * level);
        if let Some(p) = next {
            if style.depth > 0.0 {
                let (mut lo, mut hi) = (SHADOW_FALLOFF_MIN, SHADOW_FALLOFF_MAX);
                for _ in 0..24 {
                    let mid = (lo + hi) * 0.5;
                    if harmonigraph_scene::standoff_level(mid, 0.5) < p.y / style.depth {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                style.falloff = (lo + hi) * 0.5;
            }
        }
    } else {
        // A schematic caster and blur width, not a fabricated Gaussian opacity
        // profile: the actual blur depends on the caster's geometry.
        let (_, next) = plot.handle(ui, "Caster spread", style.spread, 0.15);
        if let Some(p) = next {
            style.spread = p.x;
        }
        super::weak(ui, "Blur depends on the shape casting it");
    }
    let (unit, suffix) =
        if lattice { (100.0, "%") } else { (harmonigraph_render::SPECTRAL_WIDTH_POINTS, " pt") };
    number(ui, &mut style.width, 0.0..=max, "Shadow width", unit, suffix);
    number(ui, &mut style.depth, 0.0..=1.0, "Shadow darkness", 100.0, "%");
    if style.kernel.is_distance() {
        number(
            ui,
            &mut style.falloff,
            SHADOW_FALLOFF_MIN..=SHADOW_FALLOFF_MAX,
            "Shadow falloff",
            1.0,
            "",
        );
    } else {
        number(ui, &mut style.spread, 0.0..=1.0, "Shadow spread", 100.0, "%");
    }
    plot.line(
        ui,
        vec![
            plot.point(0.0, style.depth),
            plot.point((style.width / max).sqrt(), style.depth),
            plot.point((style.width / max).sqrt(), 0.0),
        ],
        theme::hairline(),
    );
    plot.dot(ui, (style.width / max).sqrt(), style.depth);
    if style.kernel.is_distance() {
        curve(
            &plot,
            ui,
            |p| {
                (
                    p * (style.width / max).sqrt(),
                    style.depth * harmonigraph_scene::standoff_level(style.falloff, p),
                )
            },
            super::value::curve_color(),
        );
        plot.dot(
            ui,
            0.5 * (style.width / max).sqrt(),
            style.depth * harmonigraph_scene::standoff_level(style.falloff, 0.5),
        );
    } else {
        plot.line(
            ui,
            vec![plot.point(0.0, style.depth), plot.point((style.width / max).sqrt(), style.depth)],
            theme::accent(),
        );
        plot.dot(ui, style.spread, 0.15);
    }
}
