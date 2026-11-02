//! The analyzer's profile: a flat fill up to the original per-pixel FFT
//! samples and the outline along it.

use egui::{emath::Rot2, Color32, Mesh, Painter};

use super::axes::{loudness_db, power_db, spectrogram_level_db, Axes};
use super::spectrogram::cell_color;
use crate::SpectrumConfig;

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
    // only as far as the Intensity dial's luminance floor asks: a bright
    // level wears its own color, a dark one stays bright enough to read. It
    // is opaque, and never depends on the fill.
    if samples.iter().any(|&(_, d, _)| d > 0.0) {
        let floor = keyline_floor(cfg.keyline_lift);
        let points: Vec<_> = samples.iter().map(|&(t, d, _)| axes.at(t, sd(d))).collect();
        let colors: Vec<_> =
            samples.iter().map(|&(_, _, color)| keyline_color(color, floor)).collect();
        let pixel = 1.0 / painter.pixels_per_point();
        painter.add(stroke_mesh(&points, &colors, KEYLINE_WIDTH_PT, pixel));
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

/// The outline's width, in points: finer than [`crate::theme::LINE_WIDTH`], so
/// the outline does not outweigh the spectrum it traces.
const KEYLINE_WIDTH_PT: f32 = 1.0;

/// The Intensity dial, gamma-encoded so it reads evenly, as the linear luminance
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

/// The least Intensity the handover is drawn at, as a gamma fraction: about the
/// grey ruling it was drawn in before it took the outline's color.
const HANDOVER_MIN_LIFT: f32 = 0.16;

/// The spectrum/roll handover's color: the keyline's where the spectrum is
/// silent and its contour lies on the handover — the gradient's floor, lifted
/// like any other sample — but never lifted less than [`HANDOVER_MIN_LIFT`].
/// The boundary is always drawn, and at a low Intensity over a black floor the
/// silent keyline is black on black.
pub(super) fn handover_color(cfg: &SpectrumConfig) -> Color32 {
    let lift = cfg.keyline_lift.max(HANDOVER_MIN_LIFT);
    keyline_color(cell_color(cfg.spectrogram_gradient, 0.0), keyline_floor(lift))
}

/// The chords of a [`stroke_mesh`] join round a reversal. A line this fine is a
/// few pixels across, so an arc of 60 degree chords is already round.
const JOIN_CHORDS: usize = 3;

/// The most a bend of [`stroke_mesh`] turns in one row, in radians.
const JOIN_STEP: f32 = std::f32::consts::PI / JOIN_CHORDS as f32;

/// An open polyline as an antialiased ribbon whose color follows its points,
/// which `Shape::line` cannot do: it takes one color for the whole path.
///
/// Four vertices across: a transparent rim, the solid core's two edges, and
/// the other rim, `feather` (one pixel) apart at each side. A line thinner
/// than a pixel has no core and fades its peak instead, so it lays down the
/// same ink as a line of its true width — the same answer egui's own
/// tessellator gives.
///
/// A sharp bend is a round join: a fan of rows on the point, from square to the
/// segment arriving to square to the one leaving. A miter row there would lie
/// along the segments it joins, and a spectrum that swings between neighbouring
/// columns bends that sharply at every sample: its flanks closed to nothing
/// between pixel centers, and only its tips landed.
pub(super) fn stroke_mesh(
    points: &[egui::Pos2],
    colors: &[Color32],
    width: f32,
    feather: f32,
) -> Mesh {
    let (core, strength) =
        if width > feather { ((width - feather) / 2.0, 1.0) } else { (0.0, width / feather) };
    let across = [(-(core + feather), false), (-core, true), (core, true), (core + feather, false)];
    // Room for a reversal at every point, the most this can take rather than
    // what a curve usually does: growing from less is the churn #1103 removed.
    let mut mesh = grid_mesh((JOIN_CHORDS + 1) * points.len(), across.len());
    for (i, (&p, &color)) in points.iter().zip(colors).enumerate() {
        let normal = |a: egui::Pos2, b: egui::Pos2| (b - a).normalized().rot90();
        let into = if i > 0 { normal(points[i - 1], p) } else { egui::Vec2::ZERO };
        let out = if i + 1 < points.len() { normal(p, points[i + 1]) } else { egui::Vec2::ZERO };
        let mut row = |offset: egui::Vec2| {
            for (distance, solid) in across {
                let color =
                    if solid { color.gamma_multiply(strength) } else { Color32::TRANSPARENT };
                mesh.colored_vertex(p + offset * distance, color);
            }
        };
        // An end of the line has no bend. It has to be said: `atan2` reads the
        // signed zeros of a zero normal's products as a reversal.
        let turn = if into == egui::Vec2::ZERO || out == egui::Vec2::ZERO {
            0.0
        } else {
            (into.x * out.y - into.y * out.x).atan2(into.dot(out))
        };
        let steps = (turn.abs() / JOIN_STEP).ceil() as usize;
        if steps > 1 {
            for step in 0..=steps {
                row(Rot2::from_angle(turn * step as f32 / steps as f32) * into);
            }
        } else {
            // The miter: the averaged normal, lengthened so the ribbon keeps
            // its width through a bend.
            row((into + out).normalized() / (0.5 * turn).cos());
        }
    }
    for row in 1..(mesh.vertices.len() / across.len()) as u32 {
        for band in 0..across.len() as u32 - 1 {
            let a = (row - 1) * across.len() as u32 + band;
            let b = a + across.len() as u32;
            mesh.add_triangle(a, b, a + 1);
            mesh.add_triangle(a + 1, b, b + 1);
        }
    }
    mesh
}
