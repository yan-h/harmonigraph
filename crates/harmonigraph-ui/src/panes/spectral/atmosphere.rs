//! The analyzer's profile: a flat fill up to the original per-pixel FFT
//! samples, the backdrop above it and the outline along it.

use egui::{Color32, Mesh, Painter};

use super::axes::{loudness_db, power_db, spectrogram_level_db, Axes};
use super::spectrogram::cell_color;
use crate::{theme, Backdrop, KeylineStyle, SpectrumConfig};

/// Points of curve height below which the body is a sliver rather than a
/// shape. The fill is off entirely below it, so a sub-pixel curve does not
/// paint a hard line along the floor.
pub(super) const BODY_SLIVER_PT: f32 = 0.5;

/// The fill's alpha, held as the 8-bit value the picture actually carries
/// because that is the number the pane's tests read back off a vertex.
pub(super) const FILL_ALPHA_U8: u8 = 210;

/// Vertices across the body per sample: one on the floor, one on the measured
/// contour. Each sample's fill is one color and one alpha, so a stop between
/// the two would draw nothing different.
pub(super) const BODY_STOPS: usize = 2;

pub(super) fn draw_profile(
    painter: &Painter,
    axes: &Axes,
    cfg: &SpectrumConfig,
    visible: &[(f32, f32, f32)],
    budget: f32,
    split: f32,
) {
    if visible.len() < 2 {
        return;
    }
    let silence = power_db(0.0);
    let sd = |d| if split < 1.0 { split - d } else { d };
    let samples: Vec<_> = visible
        .iter()
        .map(|&(midi, t, level)| {
            (
                t,
                // Tilt can lift the stored zero-power floor above the plot's
                // floor at high pitches. It must not give silence a body.
                if level > silence { loudness_db(cfg, level, midi) * budget } else { 0.0 },
                cell_color(cfg.spectrogram_gradient, spectrogram_level_db(cfg, level, midi)),
            )
        })
        .collect();
    let vertex = |mesh: &mut Mesh, t, d, color| {
        mesh.colored_vertex(axes.at(t, sd(d)), color);
    };
    // The backdrop first, under the body it frames.
    let spacing = match cfg.backdrop {
        Backdrop::Off => None,
        Backdrop::Stripes => Some(cfg.backdrop_period.max(1.0) as usize),
        Backdrop::Gradient => Some(1),
    };
    if let Some(spacing) = spacing {
        let edge: Vec<_> = samples.iter().map(|&(t, d, _)| (t, d)).collect();
        painter.add(backdrop_mesh(
            &edge,
            spacing,
            cfg.backdrop_height * budget,
            theme::picture_ruling().gamma_multiply(cfg.backdrop_strength),
            |t, d| axes.at(t, sd(d)),
        ));
    }
    let connect = |mesh: &mut Mesh, rows: usize, bands: usize| {
        for row in 1..rows {
            for band in 0..bands - 1 {
                let a = ((row - 1) * bands + band) as u32;
                let b = (row * bands + band) as u32;
                mesh.add_triangle(a, b, a + 1);
                mesh.add_triangle(a + 1, b, b + 1);
            }
        }
    };

    let mut body = grid_mesh(samples.len(), BODY_STOPS);
    for &(t, d, color) in &samples {
        let alpha = if d * axes.depth_len() > BODY_SLIVER_PT { FILL_ALPHA_U8 } else { 0 };
        let fill = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha);
        for depth in [0.0, d] {
            vertex(&mut body, t, depth, fill);
        }
    }
    connect(&mut body, samples.len(), BODY_STOPS);
    painter.add(body);

    // The contour takes the color of the fill it bounds, lifted toward white
    // only as far as the Lift dial's luminance floor asks: a bright level
    // wears its own color, a dark one stays bright enough to read. It is
    // opaque, and never depends on the fill.
    if cfg.keyline_style != KeylineStyle::Off && samples.iter().any(|&(_, d, _)| d > 0.0) {
        let floor = keyline_floor(cfg.keyline_lift);
        let points: Vec<_> = samples.iter().map(|&(t, d, _)| axes.at(t, sd(d))).collect();
        let colors: Vec<_> =
            samples.iter().map(|&(_, _, color)| keyline_color(color, floor)).collect();
        let pixel = 1.0 / painter.pixels_per_point();
        painter.add(match cfg.keyline_style {
            KeylineStyle::Line => stroke_mesh(&points, &colors, KEYLINE_WIDTH_PT, pixel),
            KeylineStyle::Dots => {
                // Axis-aligned in every orientation, so the pitch axis's
                // direction is a unit vector along x or y.
                let along = (axes.at(1.0, 0.0) - axes.at(0.0, 0.0)).normalized().abs();
                let spacing = axes.pitch_len() / samples.len() as f32;
                pixel_caps_mesh(&points, &colors, along, spacing, pixel)
            }
            KeylineStyle::Off => unreachable!("an absent outline is not drawn"),
        });
    }
}

/// An empty mesh with exactly the room a grid of `rows` by `bands` vertices
/// takes, two triangles per cell, so filling it never grows a buffer. Without
/// this the profile's meshes doubled their way up from empty every frame, about
/// 3.8 MB of allocator traffic (#1103).
fn grid_mesh(rows: usize, bands: usize) -> Mesh {
    let mut mesh = Mesh::default();
    mesh.reserve_vertices(rows * bands);
    mesh.reserve_triangles(2 * rows.saturating_sub(1) * bands.saturating_sub(1));
    mesh
}

/// The backdrop over the curve's `edge` (`(pitch fraction, depth)` per sample):
/// the region above the edge and below `top`, lit `ink` at the floor and fading
/// linearly to nothing at `top`, in one pixel column of every `spacing`. A
/// `spacing` of 1 lights every column, which is the Gradient.
///
/// The fade is AFFINE in depth, so a vertex anywhere carrying its own depth's
/// alpha is exact across every triangle, and the region can follow the body's
/// own edge — the same straight segments between the same samples — rather
/// than a per-column step. The two then tile: no pixel is both, and none is
/// neither, whatever the column's offset from the pixel grid. Where a segment
/// rises through `top` it is cut at the crossing rather than clamped, since a
/// clamped vertex would lay the backdrop over the flank below it.
///
/// A stripe spans one column of the pitch axis, from halfway to the previous
/// sample to halfway to the next — the columns the samples stand for, which
/// are a device pixel each, so a stripe is one pixel wide.
fn backdrop_mesh(
    edge: &[(f32, f32)],
    spacing: usize,
    top: f32,
    ink: Color32,
    at: impl Fn(f32, f32) -> egui::Pos2,
) -> Mesh {
    let mut mesh = Mesh::default();
    if edge.len() < 2 || top <= 0.0 {
        return mesh;
    }
    let lit = |d: f32| ink.gamma_multiply((1.0 - d / top).clamp(0.0, 1.0));
    // The part of the segment `p`-`q` below `top`, as a quad up to `top`.
    let mut piece = |p: (f32, f32), q: (f32, f32)| {
        if q.0 <= p.0 || (p.1 >= top && q.1 >= top) {
            return;
        }
        let cross =
            |a: (f32, f32), b: (f32, f32)| (a.0 + (b.0 - a.0) * (top - a.1) / (b.1 - a.1), top);
        let (p, q) = match (p.1 < top, q.1 < top) {
            (true, false) => (p, cross(p, q)),
            (false, true) => (cross(p, q), q),
            _ => (p, q),
        };
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(at(p.0, p.1), lit(p.1));
        mesh.colored_vertex(at(q.0, q.1), lit(q.1));
        mesh.colored_vertex(at(q.0, top), Color32::TRANSPARENT);
        mesh.colored_vertex(at(p.0, top), Color32::TRANSPARENT);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    };
    if spacing <= 1 {
        for pair in edge.windows(2) {
            piece(pair[0], pair[1]);
        }
        return mesh;
    }
    let last = edge.len() - 1;
    // Halfway along the segment from sample `i` to its neighbour `j`.
    let mid = |i: usize, j: usize| {
        let (a, b) = (edge[i], edge[j]);
        (0.5 * (a.0 + b.0), 0.5 * (a.1 + b.1))
    };
    for i in (0..=last).step_by(spacing) {
        let before = if i > 0 { mid(i - 1, i) } else { edge[i] };
        let after = if i < last { mid(i, i + 1) } else { edge[i] };
        piece(before, edge[i]);
        piece(edge[i], after);
    }
    mesh
}

/// One solid cap at each point, all in one mesh: a device `pixel` deep and
/// one column `spacing` wide along the pitch axis (`along`), so neighbouring
/// caps tile the axis edge to edge. The profile has about one sample per
/// pixel column, but only about: the column count is rounded, and capped on a
/// very wide pane, and caps a fixed pixel wide would then skip a column
/// somewhere and leave a hole in a solid edge. Tiled, every pixel column gets
/// exactly one cap.
///
/// No antialiasing rim, on purpose. A cap one pixel deep covers exactly one
/// pixel center across the depth wherever it lands, so the GPU paints it
/// crisp, the nearest pixel to the true level; a feathered one would smear a
/// dimmer blot over two and shimmer as the level moves.
///
/// Opaque and in [`keyline_color`], the line's own rule: a single pixel at the
/// fill's edge in the fill's own color is the fill a pixel further out, so a
/// bright level needs no fade to disappear.
fn pixel_caps_mesh(
    points: &[egui::Pos2],
    colors: &[Color32],
    along: egui::Vec2,
    spacing: f32,
    pixel: f32,
) -> Mesh {
    let across = egui::Vec2::splat(1.0) - along;
    let half = 0.5 * (along * spacing + across * pixel);
    let mut mesh = Mesh::default();
    // `add_colored_rect` is four vertices and two triangles.
    mesh.reserve_vertices(4 * points.len());
    mesh.reserve_triangles(2 * points.len());
    for (&p, &color) in points.iter().zip(colors) {
        mesh.add_colored_rect(egui::Rect::from_min_max(p - half, p + half), color);
    }
    mesh
}

/// The outline's width, in points.
const KEYLINE_WIDTH_PT: f32 = 1.0;

/// The Lift dial, gamma-encoded so it reads evenly, as the linear luminance
/// floor [`keyline_color`] lifts to.
pub(super) fn keyline_floor(lift: f32) -> f32 {
    egui::ecolor::linear_f32_from_gamma_u8((lift.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// `fill` lifted toward white just far enough that its linear luminance
/// reaches `floor`, and untouched where it is already brighter. A floor of 1
/// is plain white and a floor of 0 is the fill itself.
pub(super) fn keyline_color(fill: Color32, floor: f32) -> Color32 {
    let c = egui::Rgba::from(fill);
    let luminance = 0.2126 * c.r() + 0.7152 * c.g() + 0.0722 * c.b();
    let lift = if luminance < floor { (floor - luminance) / (1.0 - luminance) } else { 0.0 };
    let up = |v: f32| v + (1.0 - v) * lift;
    egui::Rgba::from_rgb(up(c.r()), up(c.g()), up(c.b())).into()
}

/// An open polyline as an antialiased ribbon whose color follows its points,
/// which `Shape::line` cannot do: it takes one color for the whole path.
///
/// Four vertices across: a transparent rim, the solid core's two edges, and
/// the other rim, `feather` (one pixel) apart at each side. A line thinner
/// than a pixel has no core and fades its peak instead, so it lays down the
/// same ink as a line of its true width — the same answer egui's own
/// tessellator gives.
fn stroke_mesh(points: &[egui::Pos2], colors: &[Color32], width: f32, feather: f32) -> Mesh {
    let (core, strength) =
        if width > feather { ((width - feather) / 2.0, 1.0) } else { (0.0, width / feather) };
    let across = [(-(core + feather), false), (-core, true), (core, true), (core + feather, false)];
    let mut mesh = grid_mesh(points.len(), across.len());
    for (i, (&p, &color)) in points.iter().zip(colors).enumerate() {
        let normal = |a: egui::Pos2, b: egui::Pos2| (b - a).normalized().rot90();
        let into = if i > 0 { normal(points[i - 1], p) } else { egui::Vec2::ZERO };
        let out = if i + 1 < points.len() { normal(p, points[i + 1]) } else { egui::Vec2::ZERO };
        let one = if into == egui::Vec2::ZERO { out } else { into };
        let one = if one == egui::Vec2::ZERO { egui::Vec2::Y } else { one };
        // The miter: the averaged normal, lengthened so the ribbon keeps its
        // width through a bend, and capped so a spike's tip does not shoot off.
        let mitre = (into + out).normalized();
        let offset = if mitre == egui::Vec2::ZERO { one } else { mitre / mitre.dot(one).max(0.5) };
        for (distance, solid) in across {
            let color = if solid { color.gamma_multiply(strength) } else { Color32::TRANSPARENT };
            mesh.colored_vertex(p + offset * distance, color);
        }
    }
    for row in 1..points.len() as u32 {
        for band in 0..across.len() as u32 - 1 {
            let a = (row - 1) * across.len() as u32 + band;
            let b = a + across.len() as u32;
            mesh.add_triangle(a, b, a + 1);
            mesh.add_triangle(a + 1, b, b + 1);
        }
    }
    mesh
}
