//! Spatial profiles edited on a plot, with numeric fallbacks.
use super::plot::{value_bar, Plot};
use super::RangeBar;
use crate::theme;
use egui::Ui;
use harmonigraph_scene::star_plan::STAR_DEPTHS;

/// Which star property a [`depth`] control sets, one value per drawn layer.
#[derive(Clone, Copy, Default, PartialEq)]
pub(crate) enum Depth {
    #[default]
    Size,
    Spacing,
    Speed,
    Solid,
    Twinkle,
}

/// The per-layer star properties in one editor: a row choosing which one the
/// plot under it shows, then that one's [`depth`] control. The choice is a
/// view of the settings rather than one of them, held in egui's memory for
/// the session and never saved.
pub(crate) fn star_layers(
    ui: &mut Ui,
    stars: &mut harmonigraph_scene::StarSettings,
    size_scale: f32,
) {
    let id = ui.make_persistent_id("star layer property");
    let mut kind = ui.data(|d| d.get_temp::<Depth>(id)).unwrap_or_default();
    super::choice_row(
        ui,
        "Per layer",
        &mut kind,
        &[
            (Depth::Size, "Size", "Each layer's star size"),
            (Depth::Spacing, "Spacing", "How far apart each layer's stars sit"),
            (Depth::Speed, "Speed", "How fast each layer drifts"),
            (Depth::Solid, "Solid", "How much of each layer's stars is solid rather than glow"),
            (
                Depth::Twinkle,
                "Twinkle",
                "How far each layer's stars fade as one life gives way to the next",
            ),
        ],
    );
    ui.data_mut(|d| d.insert_temp(id, kind));
    let values = match kind {
        Depth::Size => &mut stars.star_size,
        Depth::Spacing => &mut stars.star_spacing_ratio,
        Depth::Speed => &mut stars.star_speed,
        Depth::Solid => &mut stars.star_solid,
        Depth::Twinkle => &mut stars.star_twinkle,
    };
    depth(ui, values, stars.star_layers, kind, size_scale);
}

/// Each drawn layer's own value, far to near, in any order: a dot per layer
/// to drag on its own; a line pressed out from the plot's empty space, which
/// sets every layer it spans onto it; and a bar beside the plot over the
/// drawn layers' whole range, which stretches and slides them together. A
/// depth `Star layers` leaves out is not shown and keeps its value.
/// `size_scale` is how many times its stored size the pane draws and shows a
/// star: the lattice's `LATTICE_STAR_SIZE_SCALE`, else 1. A spacing is a
/// multiple of the size, so it shows as stored.
pub(crate) fn depth(
    ui: &mut Ui,
    values: &mut [f32; STAR_DEPTHS],
    layers: u32,
    kind: Depth,
    size_scale: f32,
) {
    use harmonigraph_scene::{
        STAR_SIZE_MAX, STAR_SIZE_MIN, STAR_SOLID_MAX, STAR_SPACING_MAX, STAR_SPACING_MIN,
        STAR_SPEED_MAX, STAR_SPEED_MIN,
    };
    const PLOT: &str = "Set each layer on the plot: drag its dot, or press on empty space and draw a line across the layers to set them all onto it.";
    let percent: fn(f32) -> String = |share| format!("{:.0}%", share * 100.0);
    let (name, range, display, hover): (_, _, fn(f32) -> String, _) = match kind {
        Depth::Size => (
            "Star size",
            STAR_SIZE_MIN..=STAR_SIZE_MAX,
            |octaves| format!("{:.1} px", octaves.exp2()),
            "How big the stars are across, glow included, from the smallest layer to the largest. Drag an end to stretch every layer's size with it, or the middle to slide them all. Star spacing is a multiple of the size, so bigger stars sit farther apart.",
        ),
        Depth::Spacing => (
            "Star spacing",
            STAR_SPACING_MIN..=STAR_SPACING_MAX,
            |octaves| format!("{:.2}× size", octaves.exp2()),
            "How far apart the stars are, as a multiple of their layer's Star size, from the closest layer to the widest. Drag an end to stretch every layer's spacing with it, or the middle to slide them all. Every place holds a star, so wider spacing is fewer stars; the closest is as close as a star can sit and still be drawn whole.",
        ),
        Depth::Speed => (
            "Star speed",
            STAR_SPEED_MIN..=STAR_SPEED_MAX,
            percent,
            "How fast the stars drift, from the slowest layer to the fastest. Drag an end to stretch every layer's speed with it, or the middle to slide them all. A wider spread deepens the parallax; equal speeds move every layer together.",
        ),
        Depth::Solid => (
            "Solid share",
            0.0..=STAR_SOLID_MAX,
            percent,
            "The share of each star's radius at full strength, from its centre out, from the least solid layer to the most; the rest is glow, fading to the star's edge as Glow falloff says. Drag an end to stretch every layer's share with it, or the middle to slide them all.",
        ),
        Depth::Twinkle => (
            "Twinkle",
            0.0..=1.0,
            percent,
            "How far each layer's stars fade out when their lifetime ends, from the least to the most. At 100% each star fades to nothing and a new one appears somewhere else nearby. Below 100% each star stays in its place, dims only this far and changes into its next life's brightness and size. At 0% it never dims, so a layer packed tight enough to cover the sky never shows a gap. Drag an end to stretch every layer's twinkle with it, or the middle to slide them all.",
        ),
    };
    // Sizes and spacings run in octaves, the shares linearly, on the plot and
    // the bar alike; the bar's octaves are of the value as shown, so its
    // display stays a plain function of them.
    let (octaves, shown) = match kind {
        Depth::Size => (true, size_scale),
        Depth::Spacing => (true, 1.0),
        Depth::Speed | Depth::Solid | Depth::Twinkle => (false, 1.0),
    };
    let to_bar = |v: f32| if octaves { (v * shown).log2() } else { v };
    let from_bar = |u: f32| if octaves { u.exp2() / shown } else { u };
    let (low, high) = (to_bar(*range.start()), to_bar(*range.end()));
    let encode = |v: f32| (to_bar(v) - low) / (high - low);
    let decode = |p: f32| from_bar(low + p * (high - low)).clamp(*range.start(), *range.end());
    let places = harmonigraph_scene::star_plan::star_layer_depths(layers);
    let drawn: Vec<(usize, f32)> =
        (0..STAR_DEPTHS).filter_map(|k| places[k].map(|place| (k, place))).collect();
    ui.push_id(name, |ui| {
        let plot = Plot::with_fields(ui, &format!("{name} · far → near"), 1);
        // Before the dots, so a press on a dot drags the dot.
        let stroke = plot.stroke(ui);
        if let Some((from, to)) = stroke {
            let (left, right) = (from.x.min(to.x), from.x.max(to.x));
            for &(k, place) in &drawn {
                if (left..=right).contains(&place) {
                    // A line straight up a layer sets it to the pointer.
                    let t = if right > left { (place - from.x) / (to.x - from.x) } else { 1.0 };
                    values[k] = decode(from.y + (to.y - from.y) * t);
                }
            }
        }
        for (n, &(k, place)) in drawn.iter().enumerate() {
            let key = match n {
                0 => "Layer 1, farthest".to_owned(),
                n if n + 1 == drawn.len() => format!("Layer {}, nearest", n + 1),
                n => format!("Layer {}", n + 1),
            };
            let (_, next) = plot.handle(ui, &key, place, encode(values[k]));
            if let Some(p) = next {
                values[k] = decode(p.y);
            }
        }
        // A drag on the bar remaps from the layers and the pair as they stood
        // when it began, and the bar is handed the pair it produced rather
        // than one read back off the layers: read back, an end dragged past
        // the other would flatten the layers for good, and an end dragged
        // off a level set would be clamped against the end that followed it.
        let grab = ui.id().with("range grab");
        let held = ui.data(|d| d.get_temp::<RangeGrab>(grab));
        let (from, small, big) = held.map_or_else(
            || {
                let (small, big) =
                    drawn.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(s, b), &(k, _)| {
                        (s.min(to_bar(values[k])), b.max(to_bar(values[k])))
                    });
                (*values, small, big)
            },
            |held| (held.from, held.small, held.big),
        );
        let (mut a, mut b) = held.map_or((small, big), |held| held.pair);
        plot.fields(ui, |ui| {
            let response = RangeBar::new(&mut a, &mut b, low..=high, name)
                .display(display)
                .show(ui)
                .on_hover_text(format!("{hover} {PLOT}"));
            // Written back only on a change: the round trip is not exact, and
            // the values key star placement. Each layer keeps its share of the
            // range; when every layer is level they move with the end dragged.
            // A double click resets a range bar to the whole axis, which here
            // would spread every layer across it, so it does nothing.
            if response.changed() && !response.double_clicked() {
                for &(k, _) in &drawn {
                    let t = if big > small {
                        (to_bar(from[k]) - small) / (big - small)
                    } else if a != small {
                        0.0
                    } else {
                        1.0
                    };
                    values[k] = from_bar(a + (b - a) * t).clamp(*range.start(), *range.end());
                }
            }
            if response.dragged() {
                let held = RangeGrab { from, small, big, pair: (a, b) };
                ui.data_mut(|d| d.insert_temp(grab, held));
            } else {
                ui.data_mut(|d| d.remove_temp::<RangeGrab>(grab));
            }
        });
        let points: Vec<_> =
            drawn.iter().map(|&(k, place)| plot.point(place, encode(values[k]))).collect();
        plot.line(ui, points, theme::accent());
        if let Some((from, to)) = stroke {
            plot.line(ui, vec![plot.point(from.x, from.y), plot.point(to.x, to.y)], theme::text());
        }
        for &(k, place) in &drawn {
            plot.dot(ui, place, encode(values[k]));
        }
    });
}

/// A drag on a [`depth`] control's bar: the layers and their range in bar
/// units as the drag found them, and the pair the bar last produced.
#[derive(Clone, Copy, Default)]
struct RangeGrab {
    from: [f32; STAR_DEPTHS],
    small: f32,
    big: f32,
    pair: (f32, f32),
}

/// How a star looks at the farthest and the nearest drawn layer, `solid`
/// being their solid shares: both drawn with the real profile
/// ([`harmonigraph_scene::star_plan::star_profile`]), enlarged to the same
/// size so the shape reads whatever `Star size` is, with the falloff that
/// bends their glow beside them.
pub(crate) fn star_profile(ui: &mut Ui, solid: [f32; 2], falloff: &mut f32) {
    use harmonigraph_scene::star_plan::{star_falloff_bend, star_profile};
    let plot = Plot::with_fields(ui, "Star shape · far, near", 1);
    plot.fields(ui, |ui| {
        value_bar(ui, falloff, 0.0..=1.0, ["Glow falloff", "Glow falloff"], 100.0, "%");
    });
    plot.response.clone().on_hover_text(
        "One star at the farthest layer (left) and the nearest (right), enlarged to the same size: Star size sets how big they really are, and Solid how much of each is at full strength, from its centre out; the rest is glow, fading to the star's edge. Glow falloff is the shape of that glow, not its amount: at 0% it stays bright almost to the edge, at 50% it fades evenly, and at 100% it drops at once into a faint haze.",
    );
    let bend = star_falloff_bend(*falloff);
    let half = plot.rect.width() * 0.5;
    let radius = (half * 0.9).min(plot.rect.height() * 0.5);
    for (k, solid) in solid.into_iter().enumerate() {
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

/// Match the renderer's two compositing curves, including its ten-stop floor.
fn shadow_darkness(depth: f32, coverage: f32, lattice: bool) -> f32 {
    if lattice {
        depth * coverage
    } else {
        1.0 - (1.0 - depth).max(1.0 / 1024.0).powf(coverage)
    }
}

pub(crate) fn shadow(
    ui: &mut Ui,
    style: &mut harmonigraph_scene::ShadowStyle,
    max: f32,
    lattice: bool,
) {
    use harmonigraph_scene::{SHADOW_FALLOFF_MAX, SHADOW_FALLOFF_MIN};
    let plot = Plot::without_label(ui, 3);
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
    plot.response.clone().on_hover_text(
        "Shadow around a sample shape. Adjust Width, Darkness, and Falloff or Spread to preview the result. The sample is scaled to fit; the shadow on each shape depends on its size and outline.",
    );
    shadow_preview(ui, plot.rect, style.clamped(max), max, lattice);
}

/// A small CPU mesh of a rectangular caster's shadow. The Gaussian separates
/// into two one-dimensional integrals, so no texture or blur pass is needed.
fn shadow_preview(
    ui: &Ui,
    rect: egui::Rect,
    style: harmonigraph_scene::ShadowStyle,
    max: f32,
    lattice: bool,
) {
    const COLUMNS: u32 = 48;
    const ROWS: u32 = 32;
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.rect_filled(rect, 2.0, theme::text_dim());
    let size = rect.width().min(rect.height());
    let caster = egui::Rect::from_center_size(rect.center(), size * egui::vec2(0.38, 0.20));
    // Keep the sample and scale fixed as the controls move, with room for
    // the widest Gaussian plus full spread (2.5 shadow widths).
    let width = style.width / max * size * 0.15;
    if style.casts() {
        let sigma = width * 0.5;
        let spread = style.gaussian_spread_points(sigma);
        let blurred = caster.expand(spread);
        let xs: [f32; COLUMNS as usize + 1] = std::array::from_fn(|x| {
            let p = egui::lerp(rect.x_range(), x as f32 / COLUMNS as f32);
            gaussian_interval(p, blurred.left(), blurred.right(), sigma)
        });
        let ys: [f32; ROWS as usize + 1] = std::array::from_fn(|y| {
            let p = egui::lerp(rect.y_range(), y as f32 / ROWS as f32);
            gaussian_interval(p, blurred.top(), blurred.bottom(), sigma)
        });
        let mut mesh = egui::Mesh::default();
        for y in 0..=ROWS {
            for x in 0..=COLUMNS {
                let p = egui::pos2(
                    egui::lerp(rect.x_range(), x as f32 / COLUMNS as f32),
                    egui::lerp(rect.y_range(), y as f32 / ROWS as f32),
                );
                let coverage = if style.kernel.is_distance() {
                    harmonigraph_scene::standoff_level(
                        style.falloff,
                        caster.distance_to_pos(p) / width,
                    )
                } else {
                    // The renderer's GAUSSIAN_GAIN in common.wgsl.
                    (2.5 * xs[x as usize] * ys[y as usize]).min(1.0)
                };
                let darkness = shadow_darkness(style.depth, coverage, lattice);
                mesh.colored_vertex(
                    p,
                    egui::Color32::from_black_alpha((255.0 * darkness).round() as u8),
                );
                if x < COLUMNS && y < ROWS {
                    let a = y * (COLUMNS + 1) + x;
                    let b = a + COLUMNS + 1;
                    mesh.add_triangle(a, a + 1, b);
                    mesh.add_triangle(a + 1, b + 1, b);
                }
            }
        }
        painter.add(egui::Shape::mesh(mesh));
    }
    painter.rect_filled(caster, 0.0, theme::text());
}

/// Integral of a Gaussian over one side of the sample rectangle. The tanh
/// approximation of the normal CDF is ample for this small preview mesh.
fn gaussian_interval(p: f32, low: f32, high: f32, sigma: f32) -> f32 {
    let cdf = |edge: f32| {
        let x = ((edge - p) / sigma).clamp(-3.0, 3.0);
        0.5 + 0.5 * (0.797_884_6 * (x + 0.044_715 * x * x * x)).tanh()
    };
    (cdf(high) - cdf(low)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod shadow_tests {
    use super::shadow_darkness;
    #[test]
    fn spectral_preview_keeps_the_stronger_tails_and_finite_full_darkness() {
        assert!((shadow_darkness(0.91, 0.3, true) - 0.273).abs() < 1e-5);
        assert!((shadow_darkness(0.91, 0.3, false) - 0.5144066).abs() < 1e-5);
        assert_eq!(shadow_darkness(1.0, 0.0, false), 0.0);
        assert_eq!(shadow_darkness(1.0, 1.0, false), 1.0 - 1.0 / 1024.0);
    }
}
