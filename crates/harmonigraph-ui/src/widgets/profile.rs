//! Spatial profiles edited through their endpoints and curve, with numeric fallbacks.
use super::plot::{curve, value_bar, Plot};
use super::RangeBar;
use crate::theme;
use egui::Ui;
use std::ops::RangeInclusive;

/// Which star property a [`depth`] control spreads from far to near, either
/// way round: the plot's two end handles cross freely, and the bar under it
/// edits the pair's extent and keeps its direction. A size or spacing carries how many times the stored value the pane draws and
/// shows it: the lattice's `LATTICE_STAR_SIZE_SCALE`, else 1.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Depth {
    Size(f32),
    Spacing(f32),
    Speed,
}

pub(crate) fn depth(
    ui: &mut Ui,
    low: &mut f32,
    high: &mut f32,
    exponent: &mut f32,
    range: RangeInclusive<f32>,
    curve_range: RangeInclusive<f32>,
    kind: Depth,
) {
    let (name, curve_name) = match kind {
        Depth::Size(_) => ("Star size", "Size curve"),
        Depth::Spacing(_) => ("Star spacing", "Spacing curve"),
        Depth::Speed => ("Star speed", "Speed curve"),
    };
    // Sizes and spacings run in octaves, speed linearly.
    let (size, shown) = match kind {
        Depth::Size(shown) | Depth::Spacing(shown) => (true, shown),
        Depth::Speed => (false, 1.0),
    };
    ui.push_id(name, |ui| {
        let plot = Plot::with_fields(ui, &format!("{name} · far → near"), 2);
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
            *low = decode(p.y);
        }
        let (_, next) = plot.handle(ui, "Near depth", 1.0, encode(*high));
        if let Some(p) = next {
            *high = decode(p.y);
        }
        // The shape is a power of depth. Star sizes interpolate in log space;
        // speed interpolates linearly. Those are also the plotted units.
        let a = encode(*low);
        let b = encode(*high);
        let (_, next) =
            plot.handle(ui, "Depth distribution", 0.5, a + (b - a) * 0.5f32.powf(*exponent));
        if let Some(p) = next {
            if (b - a).abs() > 1e-6 {
                *exponent = (((p.y - a) / (b - a)).clamp(0.0001, 0.9999).ln() / 0.5f32.ln())
                    .clamp(*curve_range.start(), *curve_range.end());
            }
        }
        // The bar holds the smaller end on its left whichever depth has it, and
        // says when that is the near one.
        let reversed = *low > *high;
        let label = if reversed { format!("{name}, reversed") } else { name.to_owned() };
        let (mut small, mut big) = if reversed { (*high, *low) } else { (*low, *high) };
        let mut put = |small: f32, big: f32| {
            (*low, *high) = if reversed { (big, small) } else { (small, big) };
        };
        plot.fields(ui, |ui| {
            if size {
                // Dragged in octaves, like the plot, so the small end has room
                // on the track. Written back only on a change: the round trip
                // is not exact, and the values key star placement.
                // The octaves are of the value as shown, so the bar's own
                // display can stay a plain function of them.
                let octaves = |v: f32| (v * shown).log2();
                let (mut a, mut b) = (octaves(small), octaves(big));
                let response = RangeBar::new(
                    &mut a,
                    &mut b,
                    octaves(*range.start())..=octaves(*range.end()),
                    &label,
                )
                .display(|octaves| format!("{:.1} px", octaves.exp2()))
                .show(ui)
                .on_hover_text(if matches!(kind, Depth::Size(_)) {
                    "How big the stars are across, glow included, from the smallest to the largest. The plot above says which depth gets which: drag its near handle below the far one to make the nearest stars the smallest. One value is one size at every depth. A depth whose stars would not fit its spacing is drawn smaller, and a note below the Stars controls says so; wider spacing leaves room for bigger stars."
                } else {
                    "How far apart the stars are, from the closest to the widest. The plot above says which depth gets which: drag its near handle below the far one to make the nearest stars the densest. Every place holds a star, so wider spacing is fewer stars."
                });
                if response.changed() {
                    put(a.exp2() / shown, b.exp2() / shown);
                }
            } else {
                let response = RangeBar::new(&mut small, &mut big, range.clone(), &label)
                    .display(|speed| format!("{:.0}%", speed * 100.0))
                    .show(ui)
                    .on_hover_text(
                        "How fast the stars drift, from the slowest to the fastest. The plot above says which depth gets which: drag its near handle below the far one to make the nearest stars the slowest. A wider range deepens the parallax; equal ends move every depth together.",
                    );
                if response.changed() {
                    put(small, big);
                }
            }
            value_bar(ui, exponent, curve_range, [curve_name, "Curve"], 1.0, "");
        });
        let a = encode(*low);
        let b = encode(*high);
        curve(&plot, ui, |p| (p, a + (b - a) * p.powf(*exponent)), theme::accent());
        plot.dot(ui, 0.0, a);
        plot.dot(ui, 1.0, b);
        plot.dot(ui, 0.5, a + (b - a) * 0.5f32.powf(*exponent));
    });
}

/// How a star looks at the farthest and the nearest depth: both drawn with
/// the real profile ([`harmonigraph_scene::star_plan::star_profile`]),
/// enlarged to the same size so the shape reads whatever `Star size` is, with
/// the three bars that set it beside them.
pub(crate) fn star_profile(
    ui: &mut Ui,
    solid_far: &mut f32,
    solid_near: &mut f32,
    falloff: &mut f32,
) {
    use harmonigraph_scene::star_plan::{star_falloff_bend, star_profile};
    use harmonigraph_scene::STAR_SOLID_MAX;
    let plot = Plot::with_fields(ui, "Star shape · far, near", 3);
    plot.fields(ui, |ui| {
        let solid = 0.0..=STAR_SOLID_MAX;
        value_bar(
            ui,
            solid_far,
            solid.clone(),
            ["Solid share, far stars", "Far solid"],
            100.0,
            "%",
        );
        value_bar(ui, solid_near, solid, ["Solid share, near stars", "Near solid"], 100.0, "%");
        value_bar(ui, falloff, 0.0..=1.0, ["Glow falloff", "Glow falloff"], 100.0, "%");
    });
    plot.response.clone().on_hover_text(
        "One star at the farthest depth (left) and the nearest (right), enlarged to the same size: Star size sets how big they really are, and the depths between follow the Star size curve. Far solid and Near solid are the share of each star's radius at full strength, from its centre out; the rest is glow, fading to the star's edge. Glow falloff is the shape of that glow, not its amount: at 0% it stays bright almost to the edge, at 50% it fades evenly, and at 100% it drops at once into a faint haze.",
    );
    let bend = star_falloff_bend(*falloff);
    let half = plot.rect.width() * 0.5;
    let radius = (half * 0.9).min(plot.rect.height() * 0.5);
    for (k, solid) in [*solid_far, *solid_near].into_iter().enumerate() {
        let centre = egui::pos2(plot.rect.left() + half * (k as f32 + 0.5), plot.rect.center().y);
        star(ui, centre, radius, |t| star_profile(t, solid, bend));
    }
}

/// One star as rings of light fading by `coverage` of the distance over its
/// radius, in the panel's text colour on the plot's well.
fn star(ui: &Ui, centre: egui::Pos2, radius: f32, coverage: impl Fn(f32) -> f32) {
    const RINGS: u32 = 32;
    const SPOKES: u32 = 64;
    let light = theme::text();
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(centre, light.gamma_multiply(coverage(0.0)));
    for ring in 1..=RINGS {
        let t = ring as f32 / RINGS as f32;
        let color = light.gamma_multiply(coverage(t).clamp(0.0, 1.0));
        for spoke in 0..SPOKES {
            let angle = std::f32::consts::TAU * spoke as f32 / SPOKES as f32;
            mesh.colored_vertex(centre + radius * t * egui::vec2(angle.cos(), angle.sin()), color);
        }
    }
    let at = |ring: u32, spoke: u32| 1 + (ring - 1) * SPOKES + spoke % SPOKES;
    for spoke in 0..SPOKES {
        mesh.add_triangle(0, at(1, spoke), at(1, spoke + 1));
        for ring in 1..RINGS {
            let (a, b) = (at(ring, spoke), at(ring, spoke + 1));
            let (c, d) = (at(ring + 1, spoke), at(ring + 1, spoke + 1));
            mesh.add_triangle(a, c, b);
            mesh.add_triangle(b, c, d);
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
}

pub(crate) fn shadow(
    ui: &mut Ui,
    style: &mut harmonigraph_scene::ShadowStyle,
    max: f32,
    lattice: bool,
) {
    use harmonigraph_scene::{SHADOW_FALLOFF_MAX, SHADOW_FALLOFF_MIN};
    let plot = Plot::with_fields(ui, "Shadow profile", 3);
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
    }
    let (unit, suffix) =
        if lattice { (100.0, "%") } else { (harmonigraph_render::SPECTRAL_WIDTH_POINTS, " pt") };
    plot.fields(ui, |ui| {
        value_bar(ui, &mut style.width, 0.0..=max, ["Shadow width", "Width"], unit, suffix);
        value_bar(ui, &mut style.depth, 0.0..=1.0, ["Shadow darkness", "Darkness"], 100.0, "%");
        if style.kernel.is_distance() {
            value_bar(
                ui,
                &mut style.falloff,
                SHADOW_FALLOFF_MIN..=SHADOW_FALLOFF_MAX,
                ["Shadow falloff", "Falloff"],
                1.0,
                "",
            );
        } else {
            value_bar(ui, &mut style.spread, 0.0..=1.0, ["Shadow spread", "Spread"], 100.0, "%");
        }
    });
    plot.response.clone().on_hover_text("Drag the corner for width and darkness. Blur extent is schematic: its profile depends on the shape casting it.");
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
