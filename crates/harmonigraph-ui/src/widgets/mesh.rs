//! A band of color across a rect, rounded and feathered by its own mesh.
//!
//! The one shape in a settings pane that has to soften its own edges: epaint
//! feathers what it tessellates itself and a mesh arrives already triangulated,
//! so a band drawn edge to edge would be the only thing beside the bars with a
//! hard edge unless it builds the ring itself.
//!
//! It has no `mod tests` of its own. What it draws is read back through the
//! bands the [`SpectrumBar`](super::gradient::SpectrumBar), the
//! [`GradientPreview`](super::gradient::GradientPreview) and a
//! [`fade_span`](super::range::RangeBar::fade_span) bar paint, so the readings
//! below are `pub(super)` and the claims that use them live with those widgets.

use egui::emath::GuiRounding as _;
use egui::Vec2;
// Named bare only by the readings below; what `gradient_strip` mixes is an
// `egui::Color32` spelled out at each of the three places it names one.
#[cfg(test)]
use egui::Color32;

/// Samples taken through each of a band's corner arcs, on top of the columns
/// the band is already drawn in.
///
/// Those columns are far too coarse to round with on their own, and the
/// preview is the case that settles it: 63 columns across the column this pane
/// opens at puts them 6pt apart, wider than the radius, so a corner crosses
/// fewer than ONE of them and is drawn from its two endpoints — a diagonal cut.
/// The hue circle's 192 are 2pt apart and give a corner three steps, which is a
/// chamfer. The feather [`gradient_strip`] carries softens the edge and not the
/// SHAPE of it: a chamfer drawn smoothly is still a corner with a slice off it,
/// so how finely the arc is sampled is the whole of whether the band is round.
///
/// Both figures move with the column, and in the direction that makes the
/// preview's case the one to size for: narrow the pane and every column narrows
/// while the radius holds, so the samples matter most where the pane is widest.
pub(super) const CORNER_SAMPLES: usize = 8;

/// A band of `segments + 1` colored columns across `rect`, each column's color
/// taken from `color` at its position along the band (0 at the left edge, 1 at
/// the right) and interpolated between columns, with the band's two ends rounded
/// to `radii`.
///
/// One builder for both bands a gradient group draws — a [`SpectrumBar`]'s
/// track, which is the hue circle lit and then dimmed either side of the
/// handle, and the [`GradientPreview`] above it, which is the pitch ramp end
/// to end — and for the ramp half of a [`fade_span`](super::range::RangeBar::fade_span) fill.
/// A quad strip written out twice is two places to get the vertex order or the
/// first-column case wrong, and the second copy is the one that quietly keeps
/// the older answer.
///
/// **The rounding is in the MESH, and that is the whole reason this is not a
/// square band inside a rounded well.** A well showing round an inset mesh is
/// the ordinary way to round colors that a quad strip cannot round itself, and
/// it costs a ring of the well's own color drawn around the content — a border
/// no other control in a settings pane wears, and at the shared
/// [`CONTROL_RADIUS`](crate::theme::CONTROL_RADIUS) of 5 a one-point inset does not
/// even cover the arc, so the band's square corners poke out through it.
/// Pinching the columns to the corner circle instead lets the colors go edge to
/// edge and round about like the fill of a [`ValueBar`] beside them.
///
/// **The band softens its own edges, because the tessellator will not soften
/// them for it.** epaint feathers every shape it builds itself — the rounded
/// rect a `ValueBar` fills with fades out across one physical pixel at its edge,
/// which is the whole of egui's antialiasing — and a mesh reaches it already
/// triangulated, so it is drawn exactly as its vertices say and its edge lands
/// on whole pixels. Against the panel that reads as a stair beside the smooth
/// bars above it, which is the one thing a band drawn edge to edge cannot
/// afford. So this builds the feather itself, the way
/// `epaint::tessellator::Path::fill` does: the outline offset half a feather
/// OUTWARD into a ring of transparent vertices, the colors pulled half a feather
/// inward, and the fade between the two. Inward as well as outward is what keeps
/// the band the size the caller asked for rather than half a pixel fatter than
/// the well beneath it.
///
/// **The two radii are separate for a reason of its own**: an end that runs into
/// another shape must not round. A fade's ramp continues the solid head it meets
/// at `low`, so that end is given a radius of 0 and the head's own `rect_filled`
/// draws the corner they share.
///
/// [`ValueBar`]: super::value::ValueBar
/// [`SpectrumBar`]: super::gradient::SpectrumBar
/// [`GradientPreview`]: super::gradient::GradientPreview
pub(super) fn gradient_strip(
    painter: &egui::Painter,
    rect: egui::Rect,
    segments: usize,
    radii: (f32, f32),
    color: impl Fn(f32) -> egui::Color32,
) {
    // On the pixel grid, because a `rect_filled` is put there before it is
    // tessellated (`TessellationOptions::round_rects_to_pixels`) and a mesh is
    // not. Half a physical pixel of offset is all it takes for a band to fade
    // out across two pixels where the well under it and the bars beside it fade
    // across one, which is a blurred edge rather than a hard one and reads as
    // the same wrongness. The bands share their rect with that well, so they
    // land on the grid it does.
    let rect = rect.round_to_pixels(painter.ctx().pixels_per_point());
    let cap = (rect.height() * 0.5).min(rect.width() * 0.5);
    let (left_r, right_r) = (radii.0.clamp(0.0, cap), radii.1.clamp(0.0, cap));
    let mut xs: Vec<f32> =
        (0..=segments).map(|i| rect.left() + rect.width() * i as f32 / segments as f32).collect();
    // A squared end has no arc to sample, and asking for one puts a whole run
    // of samples on the end itself for the loop below to throw away again.
    //
    // Spread around the ARC rather than along the axis. The profile is steepest
    // where it meets the end, so samples spaced evenly in x put their widest
    // chord exactly there — half a point of the corner cut off at the radius a
    // bar rounds at, on the one part of the curve a reader is looking straight
    // at. Spaced evenly in angle the widest chord anywhere is a twentieth of
    // that, and the corner holds the circle to a tenth of a pixel all the way
    // round.
    for k in 1..CORNER_SAMPLES {
        let along = 1.0 - (k as f32 / CORNER_SAMPLES as f32 * std::f32::consts::FRAC_PI_2).cos();
        if left_r > 0.0 {
            xs.push(rect.left() + left_r * along);
        }
        if right_r > 0.0 {
            xs.push(rect.right() - right_r * along);
        }
    }
    xs.sort_by(f32::total_cmp);

    // One entry per column: where it stands, how far the nearer end pinches it
    // in, and the color it carries.
    let mut columns: Vec<(f32, f32, egui::Color32)> = Vec::with_capacity(xs.len());
    let mut drawn = f32::NEG_INFINITY;
    for x in xs {
        // Two samples landing on one column would build a triangle of no area,
        // which draws nothing and costs the same as one that does.
        if x - drawn < 0.01 {
            continue;
        }
        drawn = x;
        // The nearer end's own profile: each is 0 beyond its own radius, so a
        // band rounded the same both ways gets the one arc it would from a
        // single radius, and a squared end simply never pinches.
        let inset =
            corner_inset(x - rect.left(), left_r).max(corner_inset(rect.right() - x, right_r));
        let p = ((x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
        columns.push((x, inset, color(p)));
    }

    let reach = 0.5 * feather_width(painter);
    let top = |&(x, inset, _): &(f32, f32, egui::Color32)| egui::pos2(x, rect.top() + inset);
    // Which way the top edge faces over the run between two columns: out of the
    // band, so up, and tilted where the run is inside a corner.
    let facing = |a: &(f32, f32, egui::Color32), b: &(f32, f32, egui::Color32)| {
        let run = top(b) - top(a);
        Vec2::new(run.y, -run.x).normalized()
    };
    let mut mesh = egui::Mesh::default();
    for (i, column) in columns.iter().enumerate() {
        // Both ends of the band are vertical runs, whatever their radius: a
        // squared end is the bar's full height and a rounded one is what the
        // two arcs leave between them, down to nothing at a semicircular cap.
        // So the first and last columns face flat out along the axis.
        let before = if i == 0 { Vec2::new(-1.0, 0.0) } else { facing(&columns[i - 1], column) };
        let last = i + 1 == columns.len();
        let after = if last { Vec2::new(1.0, 0.0) } else { facing(column, &columns[i + 1]) };
        // A mitre, and not the average of the two: a corner offset along the
        // mean of its edges is pulled IN by the cosine of the turn, which is
        // what thins a feather to nothing at a sharp one. Dividing by the
        // squared length is epaint's own extension, and holds the ring the same
        // width all the way round.
        let mean = (before + after) * 0.5;
        let square = mean.length_sq();
        let out = if square > 1e-6 { mean / square } else { after } * reach;
        // The bottom edge faces the mirror of the top, the band's profile being
        // the same at both.
        let down = Vec2::new(out.x, -out.y);
        let (upper, lower) = (top(column), egui::pos2(column.0, rect.bottom() - column.1));
        let v = mesh.vertices.len() as u32;
        mesh.colored_vertex(upper + out, egui::Color32::TRANSPARENT);
        mesh.colored_vertex(upper - out, column.2);
        mesh.colored_vertex(lower - down, column.2);
        mesh.colored_vertex(lower + down, egui::Color32::TRANSPARENT);
        // A band of one column is a row of no width: it has no run to fill and
        // no edge to soften, and the vertices above are left standing alone.
        if columns.len() < 2 {
            continue;
        }
        // The two ends are edges like any other and take the same ring.
        if i == 0 || last {
            mesh.add_triangle(v, v + 1, v + 2);
            mesh.add_triangle(v, v + 2, v + 3);
        }
        if v > 0 {
            let w = v - 4;
            // Three quads across the gap to the column before: the fill, and
            // the feather above and below it.
            for (a, b) in [(0, 1), (1, 2), (2, 3)] {
                mesh.add_triangle(w + a, w + b, v + a);
                mesh.add_triangle(w + b, v + b, v + a);
            }
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// How wide the soft edge egui draws its own shapes with is, in points.
///
/// One physical pixel by default, and read from the context rather than assumed:
/// a host that turns feathering off gets a mesh with none either, which is what
/// makes the bands match whatever the bars beside them are doing.
fn feather_width(painter: &egui::Painter) -> f32 {
    let ctx = painter.ctx();
    let pixel = 1.0 / ctx.pixels_per_point();
    ctx.tessellation_options(
        |o| if o.feathering { o.feathering_size_in_pixels * pixel } else { 0.0 },
    )
}

/// How far a rounded band's edge is pinched in, top and bottom, at a column
/// `from_end` points from its nearer end: nothing along the straight run, and
/// the corner circle's own profile inside the last `radius`.
pub(super) fn corner_inset(from_end: f32, radius: f32) -> f32 {
    if from_end >= radius {
        return 0.0;
    }
    let across = radius - from_end.max(0.0);
    radius - (radius * radius - across * across).max(0.0).sqrt()
}

/// The columns a [`gradient_strip`] stands on, read back out of the mesh it
/// built: where the top and bottom of each one is, and the color between them.
///
/// Four vertices to a column — the feather's transparent pair outside the fill's
/// own — and the boundary runs down the middle of each pair, so a midpoint gives
/// back the point the band was laid out on and no test has to know how wide the
/// feather is. Shared by the bars' own tests and the settings pane's, which is
/// the other place that reads a preview out of a frame.
#[cfg(test)]
pub(crate) fn band_columns(mesh: &egui::Mesh) -> Vec<(egui::Pos2, egui::Pos2, Color32)> {
    let middle = |a: egui::Pos2, b: egui::Pos2| a + (b - a) * 0.5;
    mesh.vertices
        .chunks(4)
        .map(|c| (middle(c[0].pos, c[1].pos), middle(c[3].pos, c[2].pos), c[1].color))
        .collect()
}

/// What a [`gradient_strip`] band covers, as the caller asked for it.
///
/// `Mesh::calc_bounds` answers half a feather wider on every side, that ring
/// being transparent where it leaves the band — so it is the wrong reading for
/// anything comparing a band against the rect it was handed.
#[cfg(test)]
pub(crate) fn band_bounds(mesh: &egui::Mesh) -> egui::Rect {
    let mut bounds = egui::Rect::NOTHING;
    for (top, bottom, _) in band_columns(mesh) {
        bounds.extend_with(top);
        bounds.extend_with(bottom);
    }
    bounds
}

/// The colored bands the harness paints: the preview and the bar's own hue
/// circle. Everything else either draws is a rect, a line or a convex
/// polygon, so a mesh is a band and nothing else is.
#[cfg(test)]
pub(super) fn bands(shapes: &[egui::Shape]) -> Vec<egui::Mesh> {
    shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Mesh(m) => Some((**m).clone()),
            _ => None,
        })
        .collect()
}

/// The color each of a band's columns was painted in, left to right.
#[cfg(test)]
pub(super) fn band_colors(mesh: &egui::Mesh) -> Vec<egui::Color32> {
    band_columns(mesh).into_iter().map(|(_, _, color)| color).collect()
}

/// The four claims above, asked of one band.
///
/// Shared rather than repeated because the third band that needs them is a
/// different shape from the two a gradient group draws, and a copy is where
/// the two readings drift apart.
#[cfg(test)]
pub(super) fn fades_out_at_its_edges(which: &str, mesh: &egui::Mesh) {
    // What egui feathers with at the one pixel per point a test context
    // runs at — `TessellationOptions::feathering_size_in_pixels`.
    let feather = 1.0_f32;
    {
        let band = band_bounds(mesh);
        let ring = mesh.calc_bounds();
        // Signed so the far sides read the same way round as the near ones.
        for (side, reach, edge) in [
            ("left", ring.left(), band.left()),
            ("right", -ring.right(), -band.right()),
            ("top", ring.top(), band.top()),
            ("bottom", -ring.bottom(), -band.bottom()),
        ] {
            assert!(
                reach <= edge - feather * 0.5 + 1e-3,
                "{which}: the {side} edge stops at the band's own — nothing outside it fades",
            );
        }
        // The ring closed across the two ends, and not merely built there.
        // A band's end is one edge of the outline like any other, but it is
        // the only one whose ring quad stands inside a single column, every
        // other one spanning the gap to the next — so a triangle with all
        // three corners on the end column is exactly that quad, and nothing
        // else in the mesh can be mistaken for it.
        let last = (mesh.vertices.len() as u32 / 4 - 1) * 4;
        for (side, base) in [("near", 0), ("far", last)] {
            let closed =
                mesh.indices.chunks(3).any(|tri| tri.iter().all(|i| (base..base + 4).contains(i)));
            assert!(
                closed,
                "{which}: the {side} end has its ring of vertices and no triangle across it",
            );
        }
        for vertex in &mesh.vertices {
            assert!(
                vertex.color == Color32::TRANSPARENT || band.expand(1e-3).contains(vertex.pos),
                "{which}: color at {:?} stands outside the band {band:?}",
                vertex.pos,
            );
        }
        let outline = band_columns(mesh);
        // A column standing on the flat top of the band, corner behind it.
        let flat = |i: usize| (outline[i].0.y - band.top()).abs() < 1e-3;
        let mut straight = 0;
        for (i, column) in mesh.vertices.chunks(4).enumerate() {
            let [out_top, in_top, in_bottom, out_bottom] = column else {
                panic!("{which}: {} vertices is not whole columns", mesh.vertices.len());
            };
            assert_eq!(out_top.color, Color32::TRANSPARENT, "{which}: an outer vertex is lit");
            assert_eq!(out_bottom.color, Color32::TRANSPARENT, "{which}: an outer vertex is lit",);
            assert_eq!(in_top.color, in_bottom.color, "{which}: a column is two colors");
            assert!(in_top.color.a() > 0, "{which}: a column carries no color at all");
            // Well past the corners: a column whose NEIGHBOURS are flat too,
            // so the two edges its offset splits face the same way and the
            // direction is not a mitre's answer to a turn. The column where
            // the arc lands on the straight run is flat itself and tilts,
            // which is epaint's own behaviour at a corner and not a claim to
            // make here.
            if i > 0 && i + 1 < outline.len() && flat(i - 1) && flat(i) && flat(i + 1) {
                straight += 1;
                for (end, outer, inner, away) in
                    [("top", out_top, in_top, -feather), ("bottom", out_bottom, in_bottom, feather)]
                {
                    let across = outer.pos - inner.pos;
                    assert!(
                        across.x.abs() < 1e-3 && (across.y - away).abs() < 1e-3,
                        "{which}: the {end} edge fades by {across:?}, not {away} straight out",
                    );
                }
            }
        }
        assert!(straight > 0, "{which}: the corners ate the band, so no edge was read");
    }
}
