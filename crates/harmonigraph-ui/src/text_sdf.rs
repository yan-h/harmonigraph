//! The fixed signed-distance sheet behind lattice name shadows.
//!
//! Visible text stays on egui's own coverage atlas. This sheet carries only
//! the distance the shadow atlas needs, for the closed alphabet a lattice name
//! can emit. It is generated from the same bundled face and drawn-mark
//! rasterizer on every shell, so the editor and offline renderer do not have a
//! second asset or a second outline to keep aligned.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use glam::Vec2;
use harmonigraph_scene::BEYOND_RAMP;

use crate::marks::{mark_key, rasterize_mark, MarkKind, MARK_WEIGHT};

/// The outline is rasterized at sixteen times the lattice name's 30-point em.
const SOURCE_EM: f32 = 480.0;
/// The near field keeps sub-pixel placement accurate around the zero contour.
pub(crate) const NEAR_TEXELS_PER_EM: f32 = 64.0;
pub(crate) const NEAR_PAD: u32 = harmonigraph_render::GLYPH_SDF_NEAR_PAD;
/// The coarse field carries the smooth far range without making the near tile
/// many thousands of source pixels across. Sixteen samples per em keep its
/// conservative contour within a twentieth of an em of the near field.
pub(crate) const COARSE_TEXELS_PER_EM: f32 = 16.0;
pub(crate) const COARSE_PAD: u32 = harmonigraph_render::GLYPH_SDF_COARSE_PAD;
const SHEET_WIDTH: u32 = 1024;

/// How far outside the ink the bake looks for a second feature, in near-level
/// texels: half an em.
///
/// Past it the term is under `e^-6` at any Shadow width the picture is drawn
/// at, which is less than the f16 channel it would be stored in can tell from
/// nothing. It is also the reach a kept foot is capped at, so a texel that
/// keeps one always stores a distance the channel can hold.
const SECOND_REACH: f32 = 0.5 * NEAR_TEXELS_PER_EM;

/// What the B and A channels hold where the bake keeps no second foot.
///
/// A cosine of -1 is a foot the facing ramp weights at zero whatever
/// [`BEYOND_RAMP`] is set to, so the sentinel says "nothing to add" rather than
/// relying on the distance beside it. Both halves are finite and in range on
/// purpose: the sheet is read through a bilinear tap, and an infinity would
/// spread the sentinel over the whole texel it neighbours instead of fading
/// into it.
const NO_SECOND_FOOT: [f32; 2] = [SECOND_REACH, -1.0];

/// The coverage a texel counts as ink at, on the 0..255 scale the source
/// bitmap carries: half, which is the level [`signed_distance`] puts its zero
/// on and therefore the level the contour has to be traced at.
const INK_LEVEL: f32 = 128.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SdfPatch {
    /// Atlas texels corresponding to the visible glyph rect, min then max.
    pub(crate) near: [f32; 4],
    pub(crate) coarse: [f32; 4],
}

/// One process-wide sheet. A separate offline process generates the same
/// bytes from the same bundled inputs.
pub(crate) struct SdfSheet {
    pub(crate) atlas: harmonigraph_render::GlyphSdfAtlas,
    type_patches: HashMap<char, SdfPatch>,
    mark_patches: HashMap<MarkKind, SdfPatch>,
}

impl SdfSheet {
    pub(crate) fn type_patch(&self, family: &egui::FontFamily, ch: char) -> Option<SdfPatch> {
        match family {
            egui::FontFamily::Monospace => self.type_patches.get(&ch).copied(),
            _ => None,
        }
    }

    pub(crate) fn mark_patch(&self, kind: MarkKind) -> SdfPatch {
        self.mark_patches[&kind]
    }
}

pub(crate) fn sheet() -> &'static SdfSheet {
    static SHEET: OnceLock<SdfSheet> = OnceLock::new();
    SHEET.get_or_init(build_sheet)
}

/// Pay the deterministic generation cost during theme setup rather than on
/// the first frame that happens to show a note name.
pub(crate) fn prepare() {
    let _ = sheet();
}

#[derive(Clone)]
struct SourceGlyph {
    size: [usize; 2],
    coverage: Vec<u8>,
    /// Source-pixel rect the visible ink rect maps onto, min then max.
    map: [f32; 4],
}

/// One packed level of one glyph: [`CHANNELS`] floats per texel, in the sheet's
/// own layout.
struct Level {
    size: [u32; 2],
    pixels: Vec<f32>,
    /// Local texel coordinates corresponding to the source bitmap.
    ink: [f32; 4],
}

/// Floats per sheet texel, as the renderer uploads them
/// (`harmonigraph_render::GlyphSdfAtlas`).
///
/// What this file decides is what goes in them: R the signed distance in this
/// level's own texels, G the ANGLE of the offset to the nearest contour point,
/// B the distance to the nearest SECOND feature facing the texel across that
/// offset, in the same texels as R, and A the cosine that says how squarely it
/// faces. An angle and not a pair of components, because a vector's components
/// straddling a medial axis average to a direction pointing along neither, and
/// the sheet is read through a bilinear tap; five numbers into four channels is
/// what pays for it. The consumer applies the facing ramp to A, so the sheet
/// itself does not carry [`BEYOND_RAMP`]'s value — only its cut-off, which is
/// where a foot stops being stored at all. The coarse level carries R alone.
pub(crate) const CHANNELS: usize = harmonigraph_render::GLYPH_SDF_CHANNELS;

#[derive(Default)]
struct FloatAtlas {
    pixels: Vec<f32>,
    height: u32,
    shelf: (u32, u32, u32),
}

impl FloatAtlas {
    fn put(&mut self, level: &Level) -> [f32; 4] {
        let [w, h] = level.size;
        let (mut top, mut row_h, mut x) = self.shelf;
        if x + w > SHEET_WIDTH {
            top += row_h;
            row_h = 0;
            x = 0;
        }
        let rows = top + h;
        if rows > self.height {
            self.pixels.resize((SHEET_WIDTH * rows) as usize * CHANNELS, 0.0);
            self.height = rows;
        }
        for y in 0..h as usize {
            let from = y * w as usize * CHANNELS;
            let to = ((top as usize + y) * SHEET_WIDTH as usize + x as usize) * CHANNELS;
            let run = w as usize * CHANNELS;
            self.pixels[to..to + run].copy_from_slice(&level.pixels[from..from + run]);
        }
        self.shelf = (top, row_h.max(h), x + w);
        [
            x as f32 + level.ink[0],
            top as f32 + level.ink[1],
            x as f32 + level.ink[2],
            top as f32 + level.ink[3],
        ]
    }
}

fn build_sheet() -> SdfSheet {
    let mut atlas = FloatAtlas::default();
    let mut type_patches = HashMap::new();
    for (ch, source) in type_sources() {
        type_patches.insert(ch, pack_source(&mut atlas, &source));
    }

    let mut mark_patches = HashMap::new();
    for kind in MarkKind::ALL {
        let image = rasterize_mark(mark_key(kind, SOURCE_EM, MARK_WEIGHT, 1.0));
        let pad = crate::marks::MARK_BITMAP_PAD;
        let size = [image.size[0] - 2 * pad, image.size[1] - 2 * pad];
        let mut coverage = Vec::with_capacity(size[0] * size[1]);
        for y in pad..image.size[1] - pad {
            coverage.extend((pad..image.size[0] - pad).map(|x| image[(x, y)].a()));
        }
        let source =
            SourceGlyph { size, coverage, map: [0.0, 0.0, size[0] as f32, size[1] as f32] };
        mark_patches.insert(kind, pack_source(&mut atlas, &source));
    }

    let image = Arc::new(atlas.pixels);
    SdfSheet {
        atlas: harmonigraph_render::GlyphSdfAtlas {
            image,
            size: [SHEET_WIDTH, atlas.height.max(1)],
            key: 1,
        },
        type_patches,
        mark_patches,
    }
}

fn pack_source(atlas: &mut FloatAtlas, source: &SourceGlyph) -> SdfPatch {
    let near = near_level(source);
    let coarse = coarse_level(source);
    SdfPatch { near: atlas.put(&near), coarse: atlas.put(&coarse) }
}

fn type_sources() -> Vec<(char, SourceGlyph)> {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx.set_pixels_per_point(1.0);
    let mut placed = Vec::new();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 1024.0));
    let raw = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
    let _ = ctx.run_ui(raw, |ui| {
        placed.clear();
        let family = egui::FontFamily::Monospace;
        for ch in lattice_type_characters() {
            let galley = ui.painter().layout_no_wrap(
                ch.to_string(),
                egui::FontId::new(SOURCE_EM, family.clone()),
                egui::Color32::WHITE,
            );
            let glyph = galley
                .rows
                .iter()
                .flat_map(|row| &row.glyphs)
                .find(|glyph| !glyph.uv_rect.is_nothing())
                .unwrap_or_else(|| panic!("the bundled monospace face has no `{ch}` glyph"));
            placed.push((ch, glyph.uv_rect));
        }
    });
    let image = ctx.fonts(|fonts| fonts.image());
    placed
        .into_iter()
        .map(|(key, uv)| {
            let [x0, y0] = uv.min.map(usize::from);
            let [x1, y1] = uv.max.map(usize::from);
            let mut coverage = Vec::with_capacity((x1 - x0) * (y1 - y0));
            for y in y0..y1 {
                let row = y * image.size[0];
                coverage.extend((x0..x1).map(|x| image.pixels[row + x].a()));
            }
            let size = [x1 - x0, y1 - y0];
            (key, SourceGlyph { size, coverage, map: [0.0, 0.0, size[0] as f32, size[1] as f32] })
        })
        .collect()
}

fn lattice_type_characters() -> Vec<char> {
    let mut chars = harmonigraph_core::NoteName::typeset_characters().to_vec();
    // The optional cents line is part of the same name run and can add a sign
    // and decimal point beside NoteName's closed spelling.
    chars.extend(['-', '.']);
    chars
}

fn near_level(source: &SourceGlyph) -> Level {
    let source_pad = (0.5 * SOURCE_EM).ceil() as usize + 2;
    let [sw, sh] = source.size;
    let (w, h) = (sw + 2 * source_pad, sh + 2 * source_pad);
    let mut inside = vec![false; w * h];
    for y in 0..sh {
        for x in 0..sw {
            inside[(y + source_pad) * w + x + source_pad] = source.coverage[y * sw + x] >= 128;
        }
    }
    let nearest = signed_distance(&inside, [w, h]);
    let contour = trace_contour(&source.coverage, [sw, sh]);
    let mut feet = Vec::new();
    let scale = SOURCE_EM / NEAR_TEXELS_PER_EM;
    let span = [sw as f32 / scale, sh as f32 / scale];
    let size = [2 * NEAR_PAD + span[0].ceil() as u32, 2 * NEAR_PAD + span[1].ceil() as u32];
    let mut pixels = Vec::with_capacity((size[0] * size[1]) as usize * CHANNELS);
    for y in 0..size[1] {
        for x in 0..size[0] {
            let sx = source_pad as f32 + (x as f32 + 0.5 - NEAR_PAD as f32) * scale - 0.5;
            let sy = source_pad as f32 + (y as f32 + 0.5 - NEAR_PAD as f32) * scale - 0.5;
            let d = sample(&nearest.distance, 1, [w, h], sx, sy) / scale;
            pixels.push(d);
            // The direction is read at ONE source pixel rather than filtered
            // out of four. Across a medial axis the two sides' offsets point
            // opposite ways and their mean points along neither — a direction
            // that would say a stroke stands where no stroke does. The nearest
            // sample is the exact answer for the pixel it lands on, and either
            // side of the axis is exact at the axis itself.
            let at = [(sx.round() as usize).min(w - 1), (sy.round() as usize).min(h - 1)];
            let [ox, oy] = nearest.offset[at[1] * w + at[0]];
            pixels.push(oy.atan2(ox));
            // Only a texel standing OUTSIDE the ink and within the reach can
            // gain a second foot: inside it the union returns the signed
            // distance itself, and past it the whole shadow is already spent.
            // R decides both cheaply enough to skip the walk; what is STORED is
            // then held to the walk's own `d1`, measured against the same
            // contour as the foot beside it rather than against R's zero.
            let second = if d > 0.0 && d < SECOND_REACH {
                let point = Vec2::new(
                    (x as f32 + 0.5 - NEAR_PAD as f32) * scale,
                    (y as f32 + 0.5 - NEAR_PAD as f32) * scale,
                );
                let two = two_distances(&contour, point, &mut feet);
                match two.second {
                    Some(foot) if two.d1 < SECOND_REACH * scale => {
                        [foot.distance / scale, foot.cos_phi]
                    }
                    _ => NO_SECOND_FOOT,
                }
            } else {
                NO_SECOND_FOOT
            };
            pixels.extend(second);
        }
    }
    Level {
        size,
        pixels,
        ink: [
            NEAR_PAD as f32 + source.map[0] / scale,
            NEAR_PAD as f32 + source.map[1] / scale,
            NEAR_PAD as f32 + source.map[2] / scale,
            NEAR_PAD as f32 + source.map[3] / scale,
        ],
    }
}

fn coarse_level(source: &SourceGlyph) -> Level {
    let [sw, sh] = source.size;
    let scale = SOURCE_EM / COARSE_TEXELS_PER_EM;
    let span = [sw as f32 / scale, sh as f32 / scale];
    let inner = [span[0].ceil() as u32, span[1].ceil() as u32];
    let size = [2 * COARSE_PAD + inner[0], 2 * COARSE_PAD + inner[1]];
    let mut inside = vec![false; (size[0] * size[1]) as usize];
    for y in 0..inner[1] {
        let y0 = (y as f32 * scale).floor() as usize;
        let y1 = (((y + 1) as f32 * scale).ceil() as usize).min(sh);
        for x in 0..inner[0] {
            let x0 = (x as f32 * scale).floor() as usize;
            let x1 = (((x + 1) as f32 * scale).ceil() as usize).min(sw);
            let ink = (y0..y1).any(|sy| (x0..x1).any(|sx| source.coverage[sy * sw + sx] >= 128));
            let at = (y + COARSE_PAD) * size[0] + x + COARSE_PAD;
            inside[at as usize] = ink;
        }
    }
    let coarse = signed_distance(&inside, [size[0] as usize, size[1] as usize]);
    Level {
        size,
        pixels: coarse.distance.iter().flat_map(|&d| [d, 0.0, 0.0, 0.0]).collect(),
        ink: [
            COARSE_PAD as f32 + source.map[0] / scale,
            COARSE_PAD as f32 + source.map[1] / scale,
            COARSE_PAD as f32 + source.map[2] / scale,
            COARSE_PAD as f32 + source.map[3] / scale,
        ],
    }
}

/// One glyph's ink boundary at the half-coverage level: closed loops of
/// sub-pixel vertices in source pixels, each carrying the direction its ink
/// faces away in.
///
/// Traced rather than read off the boundary PIXELS, which is #568 §2's second
/// fact: a staircase of boundary pixels along one flat wall offers its own
/// neighbours as second features, three quarters of a point along the same
/// face, and the rule would then double a lone edge's shadow.
struct Contour {
    loops: Vec<Loop>,
}

/// One closed loop, as the edges leaving each of its vertices in turn.
///
/// One record per edge rather than a column each, because the walk below reads
/// every field of one edge together and does it once per edge per TEXEL: split
/// into five arrays it pays five bounds checks a step.
struct Loop {
    edges: Vec<Edge>,
}

/// One edge of a loop, with everything the walk needs about it precomputed.
struct Edge {
    /// The vertex it leaves, and the run to the next one.
    at: Vec2,
    run: Vec2,
    /// One over `run`'s squared length: the reciprocal that turns a projection
    /// into a place along the edge, taken here because the division is the
    /// walk's dearest instruction and depends on nothing the texel carries.
    span: f32,
    /// Which way the ink faces away, along the edge and at [`Edge::at`]. The
    /// second is the mean of this edge's and the one before it, which is the
    /// direction a corner faces.
    normal: Vec2,
    corner: Vec2,
}

/// Marching squares on `coverage` at [`INK_LEVEL`], in source-pixel
/// coordinates whose pixel `(i, j)` covers `[i, i+1) × [j, j+1)`.
///
/// The grid is the bitmap surrounded by one ring of clear, so every crossing
/// stands between two cells and every loop closes; a contour that reached the
/// border would have to be chained as an open run instead.
fn trace_contour(coverage: &[u8], [sw, sh]: [usize; 2]) -> Contour {
    let (gw, gh) = (sw + 2, sh + 2);
    let mut level = vec![0.0f32; gw * gh];
    for y in 0..sh {
        for x in 0..sw {
            level[(y + 1) * gw + x + 1] = coverage[y * sw + x] as f32;
        }
    }
    let horizontals = (gw - 1) * gh;
    let edges = horizontals + gw * (gh - 1);
    let across = |i: usize, j: usize| j * (gw - 1) + i;
    let down = |i: usize, j: usize| horizontals + j * gw + i;

    // Where an edge's crossing stands, filled as the cells that own it emit.
    // Both cells compute it from the same pair of samples, so the two agree
    // bit for bit and the chain below can join them by edge rather than by
    // comparing coordinates.
    let mut at = vec![Vec2::ZERO; edges];
    let mut segments: Vec<(usize, usize)> = Vec::new();
    let mut leaving = vec![usize::MAX; edges];
    let cut = |at: &mut Vec<Vec2>, edge: usize, a: f32, b: f32, from: Vec2, step: Vec2| {
        at[edge] = from + step * ((INK_LEVEL - a) / (b - a));
        edge
    };
    for j in 0..gh - 1 {
        for i in 0..gw - 1 {
            let (va, vb) = (level[j * gw + i], level[j * gw + i + 1]);
            let (vd, vc) = (level[(j + 1) * gw + i], level[(j + 1) * gw + i + 1]);
            let case = u8::from(va >= INK_LEVEL)
                | u8::from(vb >= INK_LEVEL) << 1
                | u8::from(vc >= INK_LEVEL) << 2
                | u8::from(vd >= INK_LEVEL) << 3;
            if case == 0 || case == 15 {
                continue;
            }
            let corner = Vec2::new(i as f32 - 0.5, j as f32 - 0.5);
            let top = |at: &mut Vec<Vec2>| cut(at, across(i, j), va, vb, corner, Vec2::X);
            let bottom =
                |at: &mut Vec<Vec2>| cut(at, across(i, j + 1), vd, vc, corner + Vec2::Y, Vec2::X);
            let left = |at: &mut Vec<Vec2>| cut(at, down(i, j), va, vd, corner, Vec2::Y);
            let right =
                |at: &mut Vec<Vec2>| cut(at, down(i + 1, j), vb, vc, corner + Vec2::X, Vec2::Y);
            // Each arm runs so that the perpendicular `(v.y, -v.x)` of its
            // direction points at the CLEAR side. The two ambiguous cases are
            // resolved by the cell's own mean, which joins the diagonal pair
            // when the middle is ink and separates them when it is not.
            let middle = 0.25 * (va + vb + vc + vd);
            let arms: [Option<(usize, usize)>; 2] = match case {
                1 => [Some((top(&mut at), left(&mut at))), None],
                2 => [Some((right(&mut at), top(&mut at))), None],
                3 => [Some((right(&mut at), left(&mut at))), None],
                4 => [Some((bottom(&mut at), right(&mut at))), None],
                6 => [Some((bottom(&mut at), top(&mut at))), None],
                7 => [Some((bottom(&mut at), left(&mut at))), None],
                8 => [Some((left(&mut at), bottom(&mut at))), None],
                9 => [Some((top(&mut at), bottom(&mut at))), None],
                11 => [Some((right(&mut at), bottom(&mut at))), None],
                12 => [Some((left(&mut at), right(&mut at))), None],
                13 => [Some((top(&mut at), right(&mut at))), None],
                14 => [Some((left(&mut at), top(&mut at))), None],
                5 if middle >= INK_LEVEL => {
                    [Some((top(&mut at), right(&mut at))), Some((bottom(&mut at), left(&mut at)))]
                }
                5 => [Some((top(&mut at), left(&mut at))), Some((bottom(&mut at), right(&mut at)))],
                10 if middle >= INK_LEVEL => {
                    [Some((left(&mut at), top(&mut at))), Some((right(&mut at), bottom(&mut at)))]
                }
                _ => [Some((right(&mut at), top(&mut at))), Some((left(&mut at), bottom(&mut at)))],
            };
            for (from, to) in arms.into_iter().flatten() {
                leaving[from] = segments.len();
                segments.push((from, to));
            }
        }
    }

    let mut walked = vec![false; segments.len()];
    let mut loops = Vec::new();
    for start in 0..segments.len() {
        if walked[start] {
            continue;
        }
        let mut points = Vec::new();
        let mut arm = start;
        while !walked[arm] {
            walked[arm] = true;
            points.push(at[segments[arm].0]);
            arm = leaving[segments[arm].1];
        }
        // A crossing that lands exactly on a grid sample gives two arms the
        // same point; collapsing them here is the plateau collapse the local
        // minima below would otherwise have to make, and it leaves every edge
        // with a length to take a normal from.
        points.dedup();
        while points.len() > 1 && points[0] == points[points.len() - 1] {
            points.pop();
        }
        if points.len() < 3 {
            continue;
        }
        let points = thin(&points);
        let n = points.len();
        let runs: Vec<Vec2> = (0..n).map(|k| points[(k + 1) % n] - points[k]).collect();
        let normals: Vec<Vec2> =
            runs.iter().map(|run| Vec2::new(run.y, -run.x).normalize_or_zero()).collect();
        let edges = (0..n)
            .map(|k| Edge {
                at: points[k],
                run: runs[k],
                span: 1.0 / runs[k].length_squared(),
                normal: normals[k],
                corner: (normals[(k + n - 1) % n] + normals[k])
                    .try_normalize()
                    .unwrap_or(normals[k]),
            })
            .collect();
        loops.push(Loop { edges });
    }
    Contour { loops }
}

/// How far apart a thinned loop's vertices may stand, in source pixels.
///
/// Marching squares puts one vertex per source pixel of contour, and the walk
/// below costs one segment test per vertex per texel, so thinning is what the
/// bake's cost is bought with. What it is paid for in is the TANGENTIAL
/// position of a foot: a second feature's foot sticks at a vertex while the
/// texel crosses that vertex's normal cone, and the cone's width grows with the
/// chord. That wobble reaches the picture only through the facing ramp, where
/// it is steepest — measured on the bar-in-a-bowl fixture, a stride of four
/// bends the consumed value 0.045 where the field it replaces bends 0.045, and
/// two bends it 0.030. Two, then, and not the four the cost alone would ask
/// for.
const CONTOUR_STRIDE: f32 = 2.0;

/// How far a thinned loop may turn between vertices, in radians.
///
/// The stride alone would cut a corner, and a stroke terminal's corner is
/// exactly what decides whether a texel gets a second foot at all. Twenty
/// degrees keeps every corner on its own vertex and still thins a smooth curve
/// by the full stride.
const CONTOUR_TURN: f32 = 0.35;

/// Drop the traced vertices a segment test cannot tell from their neighbours.
///
/// A vertex is kept once the chord from the last kept one reaches
/// [`CONTOUR_STRIDE`], or the loop has turned [`CONTOUR_TURN`] since it.
fn thin(points: &[Vec2]) -> Vec<Vec2> {
    let n = points.len();
    let mut kept = Vec::with_capacity(n);
    let mut anchor = points[0];
    let mut turn = 0.0f32;
    kept.push(anchor);
    for k in 1..n {
        turn += (points[k] - points[k - 1]).angle_to(points[(k + 1) % n] - points[k]).abs();
        if points[k].distance(anchor) >= CONTOUR_STRIDE || turn >= CONTOUR_TURN {
            kept.push(points[k]);
            anchor = points[k];
            turn = 0.0;
        }
    }
    if kept.len() < 3 {
        return points.to_vec();
    }
    kept
}

/// A second feature's foot as the sheet stores it: how far it stands, and how
/// squarely it faces.
#[derive(Clone, Copy)]
struct Foot {
    distance: f32,
    /// `cos φ` — the second foot's own direction against the direction the
    /// nearest ink points the texel in, which is what the consumer's facing
    /// ramp is applied to.
    cos_phi: f32,
}

/// What the rule reads at one point, in source pixels.
struct TwoDistances {
    /// Distance to the nearest contour point; infinite where there is no ink.
    d1: f32,
    /// The nearest foot that survives the facing test, absent where none does.
    second: Option<Foot>,
}

/// #568 §2's two distances at `x`: the nearest ink, and the nearest of the
/// caster's OTHER features that stands on or beyond the plane facing away from
/// it.
///
/// A feature is one LOCAL MINIMUM of the distance to a loop, so a smooth wall
/// offers its own nearest point and nothing else — the whole point of §2's
/// second fact. `feet` is the caller's scratch, because this runs once per
/// texel of every glyph.
fn two_distances(contour: &Contour, x: Vec2, feet: &mut Vec<(Vec2, Vec2)>) -> TwoDistances {
    feet.clear();
    let place = |edge: &Edge| ((x - edge.at).dot(edge.run) * edge.span).clamp(0.0, 1.0);
    for outline in &contour.loops {
        let mut before = place(&outline.edges[outline.edges.len() - 1]);
        for edge in &outline.edges {
            let along = place(edge);
            if along > 0.0 && along < 1.0 {
                feet.push((edge.at + edge.run * along, edge.normal));
            } else if before >= 1.0 && along <= 0.0 {
                // The distance along the loop falls into this vertex from the
                // edge before and rises out of the edge after, which is the
                // only way a vertex is a minimum; a reflex one is a maximum.
                feet.push((edge.at, edge.corner));
            }
            before = along;
        }
    }
    let mut nearest = usize::MAX;
    let mut d1 = f32::INFINITY;
    for (i, &(foot, _)) in feet.iter().enumerate() {
        let reach = foot.distance(x);
        if reach < d1 {
            d1 = reach;
            nearest = i;
        }
    }
    if nearest == usize::MAX || d1 <= 0.0 {
        return TwoDistances { d1, second: None };
    }
    let heading = (x - feet[nearest].0) / d1;
    let mut second: Option<Foot> = None;
    for (i, &(foot, out)) in feet.iter().enumerate() {
        if i == nearest {
            continue;
        }
        let offset = foot - x;
        let reach = offset.length();
        // A foot whose own ink stands between it and the texel is not a second
        // feature but the far side of the first, and the cosine test alone
        // would keep it wherever the two happen to stand square.
        if reach <= 0.0 || out.dot(-offset) <= 0.0 {
            continue;
        }
        let cos_phi = offset.dot(heading) / reach;
        if cos_phi < -BEYOND_RAMP {
            continue;
        }
        if second.is_none_or(|held| reach < held.distance) {
            second = Some(Foot { distance: reach, cos_phi });
        }
    }
    TwoDistances { d1, second }
}

/// The field one transform answers with: how far the nearest seed is, and
/// where it stands.
struct Nearest {
    /// Signed distance in source pixels, negative inside the ink.
    distance: Vec<f32>,
    /// From the pixel TO the seed it was measured against, in source pixels.
    /// Its LENGTH is the raw transform's, half a pixel off the signed distance
    /// beside it; what the sheet keeps of it is the direction alone.
    offset: Vec<[f32; 2]>,
}

/// Felzenszwalb-Huttenlocher's exact squared Euclidean distance transform,
/// once to ink and once to clear. The half-pixel correction places zero on the
/// threshold contour between the two pixel centres rather than on either one.
fn signed_distance(inside: &[bool], [w, h]: [usize; 2]) -> Nearest {
    let to_ink = edt(inside, [w, h], true);
    let to_clear = edt(inside, [w, h], false);
    let mut distance = Vec::with_capacity(w * h);
    let mut offset = Vec::with_capacity(w * h);
    for (i, &ink) in inside.iter().enumerate() {
        let from = if ink { &to_clear } else { &to_ink };
        distance.push(if ink { -(to_clear.d2[i].sqrt() - 0.5) } else { to_ink.d2[i].sqrt() - 0.5 });
        let [sx, sy] = from.site[i];
        let (x, y) = ((i % w) as i32, (i / w) as i32);
        offset.push([(sx - x) as f32, (sy - y) as f32]);
    }
    Nearest { distance, offset }
}

/// One transform's raw answer: squared distance to the nearest seed, and that
/// seed's own pixel.
struct Transform {
    d2: Vec<f32>,
    site: Vec<[i32; 2]>,
}

fn edt(mask: &[bool], [w, h]: [usize; 2], seeds: bool) -> Transform {
    const FAR: f32 = 1.0e20;
    let mut first = vec![0.0; w * h];
    // Which x each row's own transform landed on, kept for the column pass:
    // the pass below picks a ROW, and the seed is that row's winner at this
    // column. Reconstructing it from the distance instead would be a square
    // root of a sum that has already lost which side it came from.
    let mut first_site = vec![0i32; w * h];
    let mut line = vec![0.0; w.max(h)];
    let mut out = vec![0.0; w.max(h)];
    let mut sites = vec![0usize; w.max(h)];
    let mut winners = vec![0usize; w.max(h)];
    let mut bounds = vec![0.0f32; w.max(h) + 1];
    for y in 0..h {
        for x in 0..w {
            line[x] = if mask[y * w + x] == seeds { 0.0 } else { FAR };
        }
        edt_line(&line[..w], &mut out[..w], &mut sites[..w], &mut winners[..w], &mut bounds[..=w]);
        first[y * w..(y + 1) * w].copy_from_slice(&out[..w]);
        for x in 0..w {
            first_site[y * w + x] = winners[x] as i32;
        }
    }
    let mut d2 = vec![0.0; w * h];
    let mut site = vec![[0i32; 2]; w * h];
    for x in 0..w {
        for y in 0..h {
            line[y] = first[y * w + x];
        }
        edt_line(&line[..h], &mut out[..h], &mut sites[..h], &mut winners[..h], &mut bounds[..=h]);
        for y in 0..h {
            d2[y * w + x] = out[y];
            let row = winners[y];
            site[y * w + x] = [first_site[row * w + x], row as i32];
        }
    }
    Transform { d2, site }
}

fn edt_line(
    input: &[f32],
    output: &mut [f32],
    sites: &mut [usize],
    winners: &mut [usize],
    bounds: &mut [f32],
) {
    let n = input.len();
    debug_assert_eq!(output.len(), n);
    debug_assert_eq!(sites.len(), n);
    debug_assert_eq!(winners.len(), n);
    debug_assert_eq!(bounds.len(), n + 1);
    let mut k = 0usize;
    sites[0] = 0;
    bounds[0] = f32::NEG_INFINITY;
    bounds[1] = f32::INFINITY;
    for q in 1..n {
        let mut s;
        loop {
            let v = sites[k];
            s = ((input[q] + (q * q) as f32) - (input[v] + (v * v) as f32))
                / (2.0 * (q as f32 - v as f32));
            if s > bounds[k] || k == 0 {
                break;
            }
            k -= 1;
        }
        k += 1;
        sites[k] = q;
        bounds[k] = s;
        bounds[k + 1] = f32::INFINITY;
    }
    k = 0;
    for (q, value) in output.iter_mut().enumerate() {
        while bounds[k + 1] < q as f32 {
            k += 1;
        }
        let d = q as f32 - sites[k] as f32;
        *value = d * d + input[sites[k]];
        winners[q] = sites[k];
    }
}

/// Bilinear over the first channel of `values`, whose texels are `stride`
/// floats apart.
fn sample(values: &[f32], stride: usize, [w, h]: [usize; 2], x: f32, y: f32) -> f32 {
    let x = x.clamp(0.0, w.saturating_sub(1) as f32);
    let y = y.clamp(0.0, h.saturating_sub(1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let at = |x: usize, y: usize| values[(y * w + x) * stride];
    let a = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
    let b = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
    a * (1.0 - fy) + b * fy
}

#[cfg(test)]
mod tests {
    use harmonigraph_scene::crease::{facing_cosine, standoff_coverage, union_distance};
    use harmonigraph_scene::SHADOW_TAIL;

    use super::*;

    /// Source pixels to a point: [`SOURCE_EM`] is sixteen times the lattice
    /// name's thirty-point em.
    const PER_POINT: f32 = SOURCE_EM / 30.0;

    /// The Shadow width every union below is read at, in source pixels: the ten
    /// points #568's own model measures at.
    const W: f32 = 10.0 * PER_POINT;

    /// The pitch a second difference is taken on, in source pixels: the quarter
    /// point a Distance cell is drawn into at the renderer's quality floor.
    const PX: f32 = 0.25 * PER_POINT;

    /// What a smooth field's second difference comes to at [`PX`]: the
    /// curvature of the standoff's own exponential, `(SHADOW_TAIL·PX/W)²`.
    const SMOOTH: f32 = (SHADOW_TAIL * PX / W) * (SHADOW_TAIL * PX / W);

    /// How far a contour traced from a BINARY bitmap stands from the edge the
    /// fixture names, in source pixels.
    ///
    /// The half level sits `(128 - 0)/(255 - 0)` of the way from the clear
    /// sample to the ink one rather than half way, which is two thousandths of
    /// a pixel off the edge a fixture is written in whole pixels against.
    const HALF_LEVEL: f32 = 0.01;

    /// A contour traced from ink drawn by `covers`, sampled eight times a
    /// pixel each way.
    ///
    /// Antialiased and not a bare centre test, because both real sources are:
    /// egui rasterizes the face and `rasterize_mark` the drawn marks, and a
    /// binary fixture's oblique edge is a STAIRCASE whose facets alternate
    /// direction. A second foot walking one wobbles by the step's own angle,
    /// which is a property of the fixture rather than of what is measured.
    fn contour_of(size: [usize; 2], covers: impl Fn(Vec2) -> bool) -> Contour {
        const SUB: usize = 8;
        let mut coverage = vec![0u8; size[0] * size[1]];
        for y in 0..size[1] {
            for x in 0..size[0] {
                let hits = (0..SUB * SUB)
                    .filter(|s| {
                        covers(Vec2::new(
                            x as f32 + (s % SUB) as f32 / SUB as f32 + 0.5 / SUB as f32,
                            y as f32 + (s / SUB) as f32 / SUB as f32 + 0.5 / SUB as f32,
                        ))
                    })
                    .count();
                coverage[y * size[0] + x] = (255 * hits / (SUB * SUB)) as u8;
            }
        }
        trace_contour(&coverage, size)
    }

    /// What the bake stores at `x`.
    fn bake_at(contour: &Contour, x: Vec2) -> TwoDistances {
        two_distances(contour, x, &mut Vec::new())
    }

    /// The foot the walk would keep if it never asked which side of its own ink
    /// a foot stands on — the same feet, the same ramp, the same "nearest of
    /// what is left".
    fn without_backface(contour: &Contour, x: Vec2) -> Option<(Vec2, f32)> {
        let mut feet = Vec::new();
        two_distances(contour, x, &mut feet);
        let nearest = feet
            .iter()
            .enumerate()
            .min_by(|a, b| a.1 .0.distance(x).total_cmp(&b.1 .0.distance(x)))?
            .0;
        let heading = (x - feet[nearest].0).normalize();
        feet.iter()
            .enumerate()
            .filter(|(i, _)| *i != nearest)
            .filter_map(|(_, (foot, _))| {
                let offset = *foot - x;
                let reach = offset.length();
                (reach > 0.0 && offset.dot(heading) / reach >= -BEYOND_RAMP)
                    .then_some((*foot, reach))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// What a consumer reads back out of it at a facing ramp of `ramp`: the
    /// union of the two distances, carried as one and spent once.
    fn consumed(contour: &Contour, x: Vec2, ramp: f32) -> f32 {
        let two = bake_at(contour, x);
        let (d2, k) = match two.second {
            Some(foot) => (foot.distance, facing_cosine(foot.cos_phi, ramp)),
            None => (f32::INFINITY, 0.0),
        };
        standoff_coverage(union_distance(two.d1, d2, k, W), W)
    }

    /// The largest second difference of `f` along `line`.
    fn largest_bend(line: &[Vec2], f: impl Fn(Vec2) -> f32) -> f32 {
        line.windows(3)
            .map(|three| (f(three[2]) - 2.0 * f(three[1]) + f(three[0])).abs())
            .fold(0.0f32, f32::max)
    }

    /// The points from `a` to `b` inclusive, [`PX`] apart.
    fn sweep(a: Vec2, b: Vec2) -> Vec<Vec2> {
        let n = (a.distance(b) / PX).round() as usize + 1;
        (0..n).map(|i| a.lerp(b, i as f32 / (n - 1) as f32)).collect()
    }

    /// Two bars facing across a gap read half of it as their second distance,
    /// squarely.
    ///
    /// The exact case of #568 §2: on the midline both feet stand at `g/2` on
    /// the plane's own side, so the second distance is the first and the cosine
    /// is one. It is also the gate on the traced loops' ORIENTATION — the far
    /// bar's foot only survives the test that its own ink is not between it and
    /// the point if the normal the trace gave it points at the clear side.
    #[test]
    fn two_bars_read_half_their_gap_as_the_second_distance() {
        let (bar, gap) = (48.0f32, 80.0f32);
        let contour =
            contour_of([(2.0 * bar + gap) as usize, 320], |p| p.x < bar || p.x > bar + gap);
        let two = bake_at(&contour, Vec2::new(bar + gap / 2.0, 160.0));
        let foot = two.second.expect("the far bar faces the midline");
        assert!(
            (two.d1 - gap / 2.0).abs() < HALF_LEVEL,
            "the near bar stands {} away where the gap is {gap}",
            two.d1,
        );
        assert!(
            (foot.distance - gap / 2.0).abs() < HALF_LEVEL,
            "the far bar stands {} away where the gap is {gap}",
            foot.distance,
        );
        assert!(
            (foot.cos_phi - 1.0).abs() < 1.0e-5,
            "the far bar's foot stands at cos {}, not square across the gap",
            foot.cos_phi,
        );
    }

    /// An L's two arms are each other's second feature, exactly on the plane.
    ///
    /// The 90° junction is the boundary case the facing ramp has to leave at
    /// full weight, and it is on the plane by counting rather than by
    /// tolerance: the second arm's foot is perpendicular to the direction the
    /// first one points the query in, so the cosine is zero. The query stands
    /// off the diagonal on purpose — on it the two arms tie for nearest and
    /// which one is the second feature is a coin flip.
    #[test]
    fn an_l_junctions_second_foot_is_its_other_arm_on_the_plane() {
        let (thick, arm) = (48.0f32, 320.0f32);
        let contour = contour_of([arm as usize, arm as usize], |p| p.x < thick || p.y < thick);
        let x = Vec2::new(160.0, 96.0);
        let two = bake_at(&contour, x);
        let foot = two.second.expect("the upright arm faces the query");
        assert!(
            (two.d1 - (x.y - thick)).abs() < HALF_LEVEL,
            "the flat arm stands {} away where it should stand {}",
            two.d1,
            x.y - thick,
        );
        assert!(
            (foot.distance - (x.x - thick)).abs() < HALF_LEVEL,
            "the upright arm stands {} away where it should stand {}",
            foot.distance,
            x.x - thick,
        );
        assert!(
            foot.cos_phi.abs() < 1.0e-5,
            "the upright arm's foot stands at cos {}, not on the plane",
            foot.cos_phi,
        );
    }

    /// A sliced ring fills its gaps and keeps nothing at all outside itself.
    ///
    /// Both halves of #568 §2's ring row in one fixture: in the gap the two
    /// slices face each other across half of it, and outside the ring every
    /// candidate is either behind its own ink or past the ramp, so the row
    /// stays the min-distance one it was. The slices are quarter turns because
    /// that is what makes the outside claim EXACT — a narrower slice puts its
    /// own cut edge at `-sin` of its half angle, which a ramp of a half keeps
    /// at a few hundredths once the slice is under 60° wide.
    #[test]
    fn a_sliced_ring_fills_its_gaps_and_keeps_nothing_outside() {
        let (inner, outer, gap) = (192.0f32, 256.0f32, 40.0f32);
        let middle = Vec2::splat(260.0);
        let contour = contour_of([520, 520], |p| {
            let r = (p - middle).length();
            r >= inner
                && r <= outer
                && (p.x - middle.x).abs() >= gap / 2.0
                && (p.y - middle.y).abs() >= gap / 2.0
        });

        let in_gap = middle + Vec2::new((inner + outer) / 2.0, 0.0);
        let two = bake_at(&contour, in_gap);
        let foot = two.second.expect("the slice across the gap faces its middle");
        assert!(
            (foot.distance - gap / 2.0).abs() < 0.1,
            "the far slice stands {} away where the gap is {gap}",
            foot.distance,
        );
        assert!(
            foot.cos_phi > 0.999,
            "the far slice's foot stands at cos {}, not square across the gap",
            foot.cos_phi,
        );

        for out in [8.0f32, 32.0, 96.0] {
            let x = middle + Vec2::splat((outer + out) / 2.0f32.sqrt());
            let two = bake_at(&contour, x);
            assert!(
                two.second.is_none(),
                "{out} pixels outside the ring, over the middle of a slice, the bake keeps a foot \
                 {} away at cos {}",
                two.second.map_or(f32::NAN, |foot| foot.distance),
                two.second.map_or(f32::NAN, |foot| foot.cos_phi),
            );
        }
    }

    /// A solid square offers no second feature from anywhere outside it.
    ///
    /// Nothing stands beyond the plane of a convex shape, so this is the claim
    /// that keeps
    /// `a_distance_row_darkens_a_corner_as_deeply_as_an_edge_where_a_blur_retreats`
    /// green when the consumer starts reading the channel. The sweep runs from
    /// a face to a corner and from a hair's breadth out to well past the
    /// square's own width, because the two ways a far face survives — its ink
    /// between it and the query, or its cosine past the ramp — trade places
    /// with each other as the query backs away.
    #[test]
    fn a_solid_square_keeps_no_second_foot_from_outside() {
        let side = 320.0f32;
        let contour = contour_of([side as usize, side as usize], |_| true);
        for out in [1.0f32, 4.0, 16.0, 64.0, 160.0, 480.0] {
            for x in [
                Vec2::new(side / 2.0, -out),
                Vec2::new(-out, side / 2.0),
                Vec2::new(side + out, side / 3.0),
                Vec2::new(-out, -out),
                Vec2::new(side + out, -out),
                Vec2::new(side * 0.9, -out),
            ] {
                let two = bake_at(&contour, x);
                assert!(
                    two.second.is_none(),
                    "at {x} the square offers a second foot {} away at cos {}",
                    two.second.map_or(f32::NAN, |foot| foot.distance),
                    two.second.map_or(f32::NAN, |foot| foot.cos_phi),
                );
            }
        }
    }

    /// A bar ending inside a bowl reads smooth through the ramp where the hard
    /// half-plane test steps — #568 §10.5's claim, on the channels the bake
    /// stores.
    ///
    /// The sweep runs the length of the bar rather than only past its tip: a
    /// concave wall faces a tip from everywhere inside it, so the cosine never
    /// leaves 1 there and the ramp is never exercised. What crosses it is the
    /// bar's other end standing against the bowl's own terminal, and the hard
    /// test's step over the same stored pair is what says the sweep arrived.
    #[test]
    fn a_bar_end_in_a_bowl_crosses_the_ramp_without_a_step() {
        let middle = Vec2::splat(280.0);
        let (inner, outer) = (192.0f32, 256.0f32);
        let contour = contour_of([560, 560], |p| {
            let d = p - middle;
            let r = d.length();
            let bowl =
                (inner..=outer).contains(&r) && d.y.atan2(d.x).abs() <= (145.0f32).to_radians();
            let bar = (-160.0..=96.0).contains(&d.x) && d.y.abs() <= 40.0;
            bowl || bar
        });
        let line = sweep(middle + Vec2::new(176.0, 64.0), middle + Vec2::new(-256.0, 64.0));

        let (lo, hi) =
            line.iter().fold((1.0f32, -1.0f32), |(lo, hi), &x| match bake_at(&contour, x).second {
                Some(foot) => (lo.min(foot.cos_phi), hi.max(foot.cos_phi)),
                None => (lo, hi),
            });
        assert!(
            hi > 0.5 && lo < -0.25,
            "the sweep carries the second foot over cos {lo}..{hi}, which does not cross the ramp",
        );

        let hard = largest_bend(&line, |x| consumed(&contour, x, 0.0));
        let ramped = largest_bend(&line, |x| consumed(&contour, x, BEYOND_RAMP));
        let today = largest_bend(&line, |x| standoff_coverage(bake_at(&contour, x).d1, W));
        assert!(
            hard > 2.0 * today.max(SMOOTH),
            "the hard test bends {hard} where the min-distance row bends {today} and a smooth \
             field bends {SMOOTH}, so the sweep does not reach the terminal",
        );
        assert!(
            ramped <= today.max(SMOOTH),
            "the ramp bends {ramped} where the min-distance row bends {today} and a smooth field \
             bends {SMOOTH} (hard test: {hard})",
        );
    }

    /// What the walk keeps is a foot the texel can SEE: dropping the test that
    /// asks which side of its own ink a foot stands on hands the texel a
    /// nearer one with the caster between them.
    ///
    /// The cosine alone does not cover this. A foot behind its own ink can
    /// still stand square to the direction the nearest ink points the texel in,
    /// and the bowl is where that happens — its outer wall is beyond the plane
    /// from anywhere in the counter, and the inner wall in front of it is what
    /// the texel actually reads. Every point the two rules disagree at is
    /// checked, rather than one: the disagreement is the claim.
    #[test]
    fn a_foot_behind_its_own_ink_is_not_a_second_feature() {
        let middle = Vec2::splat(280.0);
        let covers = |p: Vec2| {
            let d = p - middle;
            let r = d.length();
            let bowl =
                (192.0..=256.0).contains(&r) && d.y.atan2(d.x).abs() <= (145.0f32).to_radians();
            let bar = (-160.0..=96.0).contains(&d.x) && d.y.abs() <= 40.0;
            bowl || bar
        };
        let contour = contour_of([560, 560], covers);
        let mut decided = 0usize;
        for gy in 0..140 {
            for gx in 0..140 {
                let x = Vec2::new(gx as f32 * 4.0 + 2.0, gy as f32 * 4.0 + 2.0);
                if covers(x) {
                    continue;
                }
                let kept = bake_at(&contour, x).second;
                let Some((loose, reach)) = without_backface(&contour, x) else { continue };
                if kept.is_some_and(|kept| kept.distance <= reach) {
                    continue;
                }
                decided += 1;
                // The outward normal at a foot points at the clear side, so a
                // foot the test drops has ink immediately in front of it —
                // between it and the texel that would otherwise read it.
                assert!(
                    (1..64).any(|i| covers(loose.lerp(x, i as f32 / 64.0))),
                    "at {x} the test drops a foot at {loose}, {reach} away, with no ink between \
                     the two",
                );
            }
        }
        assert!(
            decided > 100,
            "the test decides at only {decided} points of the bowl, so the fixture does not \
             reach it",
        );
    }

    /// Every source the sheet is packed from, type and marks alike.
    fn every_source() -> Vec<SourceGlyph> {
        let mut sources: Vec<SourceGlyph> =
            type_sources().into_iter().map(|(_, source)| source).collect();
        for kind in MarkKind::ALL {
            let image = rasterize_mark(mark_key(kind, SOURCE_EM, MARK_WEIGHT, 1.0));
            let pad = crate::marks::MARK_BITMAP_PAD;
            let size = [image.size[0] - 2 * pad, image.size[1] - 2 * pad];
            let mut coverage = Vec::with_capacity(size[0] * size[1]);
            for y in pad..image.size[1] - pad {
                coverage.extend((pad..image.size[0] - pad).map(|x| image[(x, y)].a()));
            }
            sources.push(SourceGlyph {
                size,
                coverage,
                map: [0.0, 0.0, size[0] as f32, size[1] as f32],
            });
        }
        sources
    }

    /// Across the whole alphabet and every mark, what the near level stores in
    /// B and A is a real second distance behind the first, or the sentinel.
    ///
    /// The fixtures above pin one reading each against an exact answer; this is
    /// the claim over the ink the sheet is actually made of, where no reading
    /// can be worked out by hand. It sweeps every texel of every level rather
    /// than sampling, because the ways a second distance goes wrong — a NaN
    /// from a degenerate loop, a foot nearer than the first, a level that fills
    /// the channel it is supposed to leave alone — each live at one texel of
    /// one glyph.
    #[test]
    fn every_character_and_mark_stores_a_second_distance_behind_its_first() {
        let mut carried = 0usize;
        for source in every_source() {
            let near = near_level(&source);
            for (i, texel) in near.pixels.chunks(CHANNELS).enumerate() {
                let (d, foot, cos_phi) = (texel[0], texel[2], texel[3]);
                let at = (i as u32 % near.size[0], i as u32 / near.size[0]);
                assert!(
                    texel.iter().all(|v| v.is_finite()),
                    "the near level holds {texel:?} at {at:?} of a {:?} glyph",
                    source.size,
                );
                assert!(
                    (-1.0..=1.0).contains(&cos_phi),
                    "the near level holds cos {cos_phi} at {at:?} of a {:?} glyph",
                    source.size,
                );
                if cos_phi <= -1.0 {
                    assert_eq!(
                        foot, NO_SECOND_FOOT[0],
                        "a texel keeping no foot at {at:?} of a {:?} glyph holds {foot} rather \
                         than the sentinel",
                        source.size,
                    );
                    continue;
                }
                carried += 1;
                assert!(
                    d > 0.0 && d < SECOND_REACH,
                    "a texel at {at:?} of a {:?} glyph carries a foot {foot} away where its own \
                     distance is {d}",
                    source.size,
                );
                // Not a tautology: the foot is measured against the traced
                // contour and R against the transform's own zero, which sits
                // between the two source pixels either side of the half level
                // rather than on it. The two conventions stand up to half a
                // source pixel apart, and the closest this comes over the whole
                // sheet is a hundredth of a texel.
                assert!(
                    foot >= d,
                    "a texel at {at:?} of a {:?} glyph carries a foot {foot} away, nearer than \
                     its own {d}",
                    source.size,
                );
            }

            let coarse = coarse_level(&source);
            assert!(
                coarse.pixels.chunks(CHANNELS).all(|texel| texel[1..] == [0.0; CHANNELS - 1]),
                "the coarse level of a {:?} glyph carries something in G, B or A",
                source.size,
            );
        }
        assert!(
            carried > 1000,
            "only {carried} texels in the whole sheet carry a second foot, so the sweep is over \
             sentinels",
        );
    }

    /// What the bake costs the one process-wide sheet, against the 0.1–0.3 s
    /// #568 §5-B2 budgets for it.
    ///
    /// The stages nest: the sheet contains the near levels, and a near level
    /// contains its own contour trace, so the rows are shares of the row below
    /// rather than a sum.
    #[test]
    #[ignore = "a probe: prints the bake's cost, asserts nothing"]
    fn the_second_distance_bake_costs_this_much() {
        // The whole sheet first and once, because it is the row that has a
        // number to be measured AGAINST — what the same call cost before the
        // bake — and a second build of it reads faster than the one the editor
        // pays for.
        let whole = std::time::Instant::now();
        let sheet = build_sheet();
        println!("| stage | seconds |");
        println!("|---|---|");
        println!("| the whole sheet | {:.3} |", whole.elapsed().as_secs_f64());

        let sources = std::time::Instant::now();
        let glyphs = every_source();
        println!("| rasterizing every source | {:.3} |", sources.elapsed().as_secs_f64());

        let mut vertices = 0usize;
        let trace = std::time::Instant::now();
        for source in &glyphs {
            let contour = trace_contour(&source.coverage, source.size);
            vertices += contour.loops.iter().map(|outline| outline.edges.len()).sum::<usize>();
        }
        println!("| tracing every contour | {:.3} |", trace.elapsed().as_secs_f64());

        let near = std::time::Instant::now();
        let levels: Vec<Level> = glyphs.iter().map(near_level).collect();
        println!("| every near level, bake included | {:.3} |", near.elapsed().as_secs_f64());
        let coarse = std::time::Instant::now();
        for source in &glyphs {
            let _ = coarse_level(source);
        }
        println!("| every coarse level | {:.3} |", coarse.elapsed().as_secs_f64());

        let texels: usize =
            levels.iter().map(|level| (level.size[0] * level.size[1]) as usize).sum();
        let carrying: Vec<&[f32]> = levels
            .iter()
            .flat_map(|level| level.pixels.chunks(CHANNELS))
            .filter(|texel| texel[3] > -1.0)
            .collect();
        let margin = carrying.iter().map(|texel| texel[2] - texel[0]).fold(f32::INFINITY, f32::min);
        println!();
        println!(
            "{} sources, {vertices} contour vertices, {texels} near texels, {} carrying a second \
             foot, the nearest of them {margin} texels behind its own first; sheet {}x{}",
            glyphs.len(),
            carrying.len(),
            sheet.atlas.size[0],
            sheet.atlas.size[1],
        );
    }

    fn field_at(near: &Level, coarse: &Level, point: [f32; 2], source: [f32; 2]) -> (f32, f32) {
        let mapped = |level: &Level| {
            let span = [level.ink[2] - level.ink[0], level.ink[3] - level.ink[1]];
            let texel = [
                level.ink[0] + point[0] / source[0] * span[0],
                level.ink[1] + point[1] / source[1] * span[1],
            ];
            let scale = 0.5 * (source[0] / span[0] + source[1] / span[1]);
            (span, texel, scale)
        };
        let sampled = |level: &Level, texel: [f32; 2], bounds: [f32; 4]| {
            let safe = [
                texel[0].clamp(bounds[0] + 0.5, bounds[2] - 0.5),
                texel[1].clamp(bounds[1] + 0.5, bounds[3] - 0.5),
            ];
            (
                sample(
                    &level.pixels,
                    CHANNELS,
                    [level.size[0] as usize, level.size[1] as usize],
                    safe[0] - 0.5,
                    safe[1] - 0.5,
                ),
                safe,
            )
        };

        let (_, near_texel, near_scale) = mapped(near);
        let (_, coarse_texel, coarse_scale) = mapped(coarse);
        let near_bounds = [
            near.ink[0] - NEAR_PAD as f32,
            near.ink[1] - NEAR_PAD as f32,
            near.ink[2] + NEAR_PAD as f32,
            near.ink[3] + NEAR_PAD as f32,
        ];
        let coarse_bounds = [
            coarse.ink[0] - COARSE_PAD as f32,
            coarse.ink[1] - COARSE_PAD as f32,
            coarse.ink[2] + COARSE_PAD as f32,
            coarse.ink[3] + COARSE_PAD as f32,
        ];
        let (near_value, _) = sampled(near, near_texel, near_bounds);
        let (coarse_value, coarse_at) = sampled(coarse, coarse_texel, coarse_bounds);
        let dx = coarse_texel[0] - coarse_at[0];
        let dy = coarse_texel[1] - coarse_at[1];
        let coarse_value = coarse_value * coarse_scale + dx.hypot(dy) * coarse_scale;
        let near_value = near_value * near_scale;
        let near_edge = (near_texel[0] - near_bounds[0] - 0.5)
            .min(near_texel[1] - near_bounds[1] - 0.5)
            .min(near_bounds[2] - 0.5 - near_texel[0])
            .min(near_bounds[3] - 0.5 - near_texel[1]);
        let t = (near_edge / harmonigraph_render::GLYPH_SDF_NEAR_BLEND as f32).clamp(0.0, 1.0);
        let t = t * t * (3.0 - 2.0 * t);
        let blended = coarse_value * (1.0 - t) + near_value * t;
        let hard = if near_edge >= 0.0 { near_value } else { coarse_value };
        (blended, hard)
    }

    #[test]
    fn the_exact_transform_puts_zero_between_ink_and_clear() {
        let d = signed_distance(&[false, true, false], [3, 1]);
        assert_eq!(d.distance, vec![0.5, -0.5, 0.5]);
    }

    /// The far field is conservative rather than contour-exact, but its edge
    /// stays within a twentieth of an em of the near field. A larger mismatch
    /// becomes a second dark shelf after the blend, even when the transition
    /// itself has no step.
    #[test]
    fn the_near_and_coarse_fields_handoff_without_a_step() {
        let (_, source) = type_sources()
            .into_iter()
            .find(|(ch, _)| *ch == 'A')
            .expect("the lattice alphabet carries A");
        let size = source.size;
        let near = near_level(&source);
        let coarse = coarse_level(&source);
        let readings: Vec<(f32, f32)> = (160..=320)
            .map(|outside| {
                field_at(
                    &near,
                    &coarse,
                    [size[0] as f32 + outside as f32, size[1] as f32 / 2.0],
                    [size[0] as f32, size[1] as f32],
                )
            })
            .collect();
        let worst = |column: usize| {
            readings
                .windows(2)
                .map(|pair| {
                    let value = |reading: &(f32, f32)| {
                        if column == 0 {
                            reading.0
                        } else {
                            reading.1
                        }
                    };
                    (value(&pair[1]) - value(&pair[0])).abs()
                })
                .fold(0.0f32, f32::max)
        };
        let hard = worst(1);
        let blended = worst(0);
        assert!(
            hard < SOURCE_EM / 20.0,
            "the near and far contours part by {hard} source pixels at their handoff"
        );
        assert!(
            blended < 4.0,
            "the blended field moves {blended} source pixels between adjacent samples"
        );
    }

    #[test]
    fn every_note_name_character_and_drawn_mark_has_an_sdf() {
        let sheet = sheet();
        for ch in harmonigraph_core::NoteName::typeset_characters() {
            assert!(sheet.type_patches.contains_key(&ch), "missing monospace `{ch}`");
        }
        for kind in MarkKind::ALL {
            assert!(sheet.mark_patches.contains_key(&kind), "missing {kind:?}");
        }
    }
}
