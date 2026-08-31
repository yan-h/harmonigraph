//! The fixed signed-distance sheet behind lattice name shadows.
//!
//! Visible text stays on egui's own coverage atlas. This sheet carries only
//! the distance the shadow atlas needs, for the closed alphabet a lattice name
//! can emit. It is generated from the same bundled face and drawn-mark
//! rasterizer on every shell, so the editor and offline renderer do not have a
//! second asset or a second outline to keep aligned.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::marks::{mark_key, rasterize_mark, MarkKind, MARK_WEIGHT};

/// The outline is rasterized at sixteen times the lattice name's 30-point em.
const SOURCE_EM: f32 = 480.0;
/// The near field keeps sub-pixel placement accurate around the zero contour.
pub(crate) const NEAR_TEXELS_PER_EM: f32 = 64.0;
pub(crate) const NEAR_PAD: u32 = harmonigraph_render::GLYPH_SDF_NEAR_PAD;
/// The coarse field carries the smooth far range without making the near tile
/// many thousands of source pixels across.
pub(crate) const COARSE_TEXELS_PER_EM: f32 = 4.0;
pub(crate) const COARSE_PAD: u32 = harmonigraph_render::GLYPH_SDF_COARSE_PAD;
const SHEET_WIDTH: u32 = 1024;

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

struct Level {
    size: [u32; 2],
    pixels: Vec<f32>,
    /// Local texel coordinates corresponding to the source bitmap.
    ink: [f32; 4],
}

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
            self.pixels.resize((SHEET_WIDTH * rows) as usize, 0.0);
            self.height = rows;
        }
        for y in 0..h as usize {
            let from = y * w as usize;
            let to = (top as usize + y) * SHEET_WIDTH as usize + x as usize;
            self.pixels[to..to + w as usize]
                .copy_from_slice(&level.pixels[from..from + w as usize]);
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
    let distance = signed_distance(&inside, [w, h]);
    let scale = SOURCE_EM / NEAR_TEXELS_PER_EM;
    let span = [sw as f32 / scale, sh as f32 / scale];
    let size = [2 * NEAR_PAD + span[0].ceil() as u32, 2 * NEAR_PAD + span[1].ceil() as u32];
    let mut pixels = Vec::with_capacity((size[0] * size[1]) as usize);
    for y in 0..size[1] {
        for x in 0..size[0] {
            let sx = source_pad as f32 + (x as f32 + 0.5 - NEAR_PAD as f32) * scale - 0.5;
            let sy = source_pad as f32 + (y as f32 + 0.5 - NEAR_PAD as f32) * scale - 0.5;
            pixels.push(sample(&distance, [w, h], sx, sy) / scale);
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
    Level {
        size,
        pixels: signed_distance(&inside, [size[0] as usize, size[1] as usize]),
        ink: [
            COARSE_PAD as f32 + source.map[0] / scale,
            COARSE_PAD as f32 + source.map[1] / scale,
            COARSE_PAD as f32 + source.map[2] / scale,
            COARSE_PAD as f32 + source.map[3] / scale,
        ],
    }
}

/// Felzenszwalb-Huttenlocher's exact squared Euclidean distance transform,
/// once to ink and once to clear. The half-pixel correction places zero on the
/// threshold contour between the two pixel centres rather than on either one.
fn signed_distance(inside: &[bool], [w, h]: [usize; 2]) -> Vec<f32> {
    let to_ink = edt(inside, [w, h], true);
    let to_clear = edt(inside, [w, h], false);
    inside
        .iter()
        .enumerate()
        .map(|(i, &ink)| if ink { -(to_clear[i].sqrt() - 0.5) } else { to_ink[i].sqrt() - 0.5 })
        .collect()
}

fn edt(mask: &[bool], [w, h]: [usize; 2], seeds: bool) -> Vec<f32> {
    const FAR: f32 = 1.0e20;
    let mut first = vec![0.0; w * h];
    let mut line = vec![0.0; w.max(h)];
    let mut out = vec![0.0; w.max(h)];
    let mut sites = vec![0usize; w.max(h)];
    let mut bounds = vec![0.0f32; w.max(h) + 1];
    for y in 0..h {
        for x in 0..w {
            line[x] = if mask[y * w + x] == seeds { 0.0 } else { FAR };
        }
        edt_line(&line[..w], &mut out[..w], &mut sites[..w], &mut bounds[..=w]);
        first[y * w..(y + 1) * w].copy_from_slice(&out[..w]);
    }
    let mut result = vec![0.0; w * h];
    for x in 0..w {
        for y in 0..h {
            line[y] = first[y * w + x];
        }
        edt_line(&line[..h], &mut out[..h], &mut sites[..h], &mut bounds[..=h]);
        for y in 0..h {
            result[y * w + x] = out[y];
        }
    }
    result
}

fn edt_line(input: &[f32], output: &mut [f32], sites: &mut [usize], bounds: &mut [f32]) {
    let n = input.len();
    debug_assert_eq!(output.len(), n);
    debug_assert_eq!(sites.len(), n);
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
    }
}

fn sample(values: &[f32], [w, h]: [usize; 2], x: f32, y: f32) -> f32 {
    let x = x.clamp(0.0, w.saturating_sub(1) as f32);
    let y = y.clamp(0.0, h.saturating_sub(1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let a = values[y0 * w + x0] * (1.0 - fx) + values[y0 * w + x1] * fx;
    let b = values[y1 * w + x0] * (1.0 - fx) + values[y1 * w + x1] * fx;
    a * (1.0 - fy) + b * fy
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(d, vec![0.5, -0.5, 0.5]);
    }

    /// The sparse far field is conservative rather than contour-exact, so a
    /// hard handoff exposes their disagreement as a ring. The shipped blend
    /// turns that mismatch into an ordinary distance gradient.
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
        assert!(hard > 20.0, "the fixture's hard handoff moved only {hard} source pixels");
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
