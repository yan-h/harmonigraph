//! The fixed signed-distance sheet behind text shadows.
//!
//! Visible text stays on egui's own coverage atlas. This sheet carries only
//! the distance the shadow atlas needs, for the closed alphabet any shadowed
//! text producer can emit. The expensive exact distance transforms are baked
//! into the binary at development time; a byte-for-byte regeneration test
//! keeps that asset tied to the bundled face, drawn-mark rasterizer, and
//! consumer alphabet.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::marks::MarkKind;
#[cfg(test)]
use crate::marks::{mark_key, rasterize_mark, MARK_WEIGHT};

const BAKED_SHEET: &[u8] = include_bytes!("../assets/text_sdf_sheet.bin");
const BAKED_MAGIC: &[u8; 8] = b"HGSDF001";

/// The outline is rasterized at sixteen times the lattice name's 30-point em.
#[cfg(test)]
const SOURCE_EM: f32 = 480.0;
/// The near field keeps sub-pixel placement accurate around the zero contour.
#[cfg(test)]
pub(crate) const NEAR_TEXELS_PER_EM: f32 = 64.0;
#[cfg(test)]
pub(crate) const NEAR_PAD: u32 = harmonigraph_render::GLYPH_SDF_NEAR_PAD;
/// The coarse field carries the smooth far range without making the near tile
/// many thousands of source pixels across. Sixteen samples per em keep its
/// conservative contour within a twentieth of an em of the near field.
#[cfg(test)]
pub(crate) const COARSE_TEXELS_PER_EM: f32 = 16.0;
#[cfg(test)]
pub(crate) const COARSE_PAD: u32 = harmonigraph_render::GLYPH_SDF_COARSE_PAD;
#[cfg(test)]
const SHEET_WIDTH: u32 = 1024;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SdfPatch {
    /// Atlas texels corresponding to the visible glyph rect, min then max.
    pub(crate) near: [f32; 4],
    pub(crate) coarse: [f32; 4],
}

/// One process-wide decoding of the bytes shared by the plugin and offline
/// renderer binaries.
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
    SHEET.get_or_init(|| {
        decode_sheet(BAKED_SHEET)
            .unwrap_or_else(|error| panic!("the bundled text SDF sheet is invalid: {error}"))
    })
}

/// Decode the baked pixels during theme setup rather than on the first frame
/// that happens to show shadowed text. This is a linear byte copy instead of
/// the old font rasterization and 52 exact distance transforms.
pub(crate) fn prepare() {
    let _ = sheet();
}

struct BakedCursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> BakedCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let end = self.at.checked_add(N).ok_or_else(|| "offset overflow".to_owned())?;
        let bytes =
            self.bytes.get(self.at..end).ok_or_else(|| format!("truncated at byte {}", self.at))?;
        self.at = end;
        Ok(bytes.try_into().expect("the slice has the requested length"))
    }

    fn u32(&mut self) -> Result<u32, String> {
        self.take().map(u32::from_le_bytes)
    }

    fn f32(&mut self) -> Result<f32, String> {
        self.take().map(f32::from_le_bytes)
    }

    fn patch(&mut self) -> Result<SdfPatch, String> {
        let mut values = [0.0; 8];
        for value in &mut values {
            *value = self.f32()?;
            if !value.is_finite() {
                return Err("patch metadata contains a non-finite coordinate".to_owned());
            }
        }
        Ok(SdfPatch {
            near: values[..4].try_into().expect("four near coordinates"),
            coarse: values[4..].try_into().expect("four coarse coordinates"),
        })
    }
}

fn decode_sheet(bytes: &[u8]) -> Result<SdfSheet, String> {
    let mut input = BakedCursor::new(bytes);
    if input.take::<8>()? != *BAKED_MAGIC {
        return Err("bad magic or unsupported version".to_owned());
    }
    let size = [input.u32()?, input.u32()?];
    let type_count = usize::try_from(input.u32()?).map_err(|_| "type count overflow")?;
    let mark_count = usize::try_from(input.u32()?).map_err(|_| "mark count overflow")?;
    let pixel_count = usize::try_from(input.u32()?).map_err(|_| "pixel count overflow")?;
    let expected_pixels = usize::try_from(size[0])
        .ok()
        .and_then(|width| {
            usize::try_from(size[1]).ok().and_then(|height| width.checked_mul(height))
        })
        .ok_or_else(|| "atlas dimensions overflow".to_owned())?;
    if pixel_count != expected_pixels {
        return Err(format!(
            "header claims {pixel_count} pixels for a {}x{} atlas",
            size[0], size[1]
        ));
    }

    let mut type_patches = HashMap::with_capacity(type_count);
    for _ in 0..type_count {
        let scalar = input.u32()?;
        let ch =
            char::from_u32(scalar).ok_or_else(|| format!("invalid character U+{scalar:04X}"))?;
        if type_patches.insert(ch, input.patch()?).is_some() {
            return Err(format!("duplicate character `{ch}`"));
        }
    }

    if mark_count != MarkKind::ALL.len() {
        return Err(format!("expected {} marks, found {mark_count}", MarkKind::ALL.len()));
    }
    let mut mark_patches = HashMap::with_capacity(mark_count);
    for (expected_tag, kind) in MarkKind::ALL.into_iter().enumerate() {
        let tag = usize::try_from(input.u32()?).map_err(|_| "mark tag overflow")?;
        if tag != expected_tag {
            return Err(format!("expected mark tag {expected_tag}, found {tag}"));
        }
        mark_patches.insert(kind, input.patch()?);
    }

    let remaining = bytes.len() - input.at;
    let expected_bytes = pixel_count
        .checked_mul(std::mem::size_of::<f32>())
        .ok_or_else(|| "pixel byte count overflow".to_owned())?;
    if remaining != expected_bytes {
        return Err(format!("expected {expected_bytes} pixel bytes, found {remaining}"));
    }
    let pixels = bytes[input.at..]
        .chunks_exact(std::mem::size_of::<f32>())
        .map(|bytes| f32::from_le_bytes(bytes.try_into().expect("one encoded f32")))
        .collect();

    Ok(SdfSheet {
        atlas: harmonigraph_render::GlyphSdfAtlas { image: Arc::new(pixels), size, key: 1 },
        type_patches,
        mark_patches,
    })
}

#[cfg(test)]
#[derive(Clone)]
struct SourceGlyph {
    size: [usize; 2],
    coverage: Vec<u8>,
    /// Source-pixel rect the visible ink rect maps onto, min then max.
    map: [f32; 4],
}

#[cfg(test)]
struct Level {
    size: [u32; 2],
    pixels: Vec<f32>,
    /// Local texel coordinates corresponding to the source bitmap.
    ink: [f32; 4],
}

#[cfg(test)]
#[derive(Default)]
struct FloatAtlas {
    pixels: Vec<f32>,
    height: u32,
    shelf: (u32, u32, u32),
}

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
fn pack_source(atlas: &mut FloatAtlas, source: &SourceGlyph) -> SdfPatch {
    let near = near_level(source);
    let coarse = coarse_level(source);
    SdfPatch { near: atlas.put(&near), coarse: atlas.put(&coarse) }
}

#[cfg(test)]
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
        for ch in shadowed_type_characters() {
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

#[cfg(test)]
fn shadowed_type_characters() -> Vec<char> {
    let mut chars = harmonigraph_core::NoteName::typeset_characters().to_vec();
    // The optional cents line is part of a lattice name run and adds a sign
    // and decimal point beside NoteName's closed spelling. Spectral frequency
    // labels add the lower-case unit suffix. Level labels need only the same
    // sign and digits already present here.
    chars.extend(['-', '.', 'k']);
    chars
}

#[cfg(test)]
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

#[cfg(test)]
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
#[cfg(test)]
fn signed_distance(inside: &[bool], [w, h]: [usize; 2]) -> Vec<f32> {
    let to_ink = edt(inside, [w, h], true);
    let to_clear = edt(inside, [w, h], false);
    inside
        .iter()
        .enumerate()
        .map(|(i, &ink)| if ink { -(to_clear[i].sqrt() - 0.5) } else { to_ink[i].sqrt() - 0.5 })
        .collect()
}

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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

    fn encode_patch(bytes: &mut Vec<u8>, patch: SdfPatch) {
        for value in patch.near.into_iter().chain(patch.coarse) {
            bytes.extend(value.to_le_bytes());
        }
    }

    fn encode_sheet(sheet: &SdfSheet) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(BAKED_MAGIC);
        bytes.extend(sheet.atlas.size[0].to_le_bytes());
        bytes.extend(sheet.atlas.size[1].to_le_bytes());
        bytes.extend(
            u32::try_from(sheet.type_patches.len()).expect("the type count fits").to_le_bytes(),
        );
        bytes.extend(
            u32::try_from(sheet.mark_patches.len()).expect("the mark count fits").to_le_bytes(),
        );
        bytes.extend(
            u32::try_from(sheet.atlas.image.len()).expect("the pixel count fits").to_le_bytes(),
        );

        let mut type_patches: Vec<_> = sheet.type_patches.iter().collect();
        type_patches.sort_unstable_by_key(|(ch, _)| **ch);
        for (&ch, &patch) in type_patches {
            bytes.extend(u32::from(ch).to_le_bytes());
            encode_patch(&mut bytes, patch);
        }
        for (tag, kind) in MarkKind::ALL.into_iter().enumerate() {
            bytes.extend(u32::try_from(tag).expect("the mark tag fits").to_le_bytes());
            encode_patch(&mut bytes, sheet.mark_patches[&kind]);
        }
        for &pixel in sheet.atlas.image.iter() {
            bytes.extend(pixel.to_le_bytes());
        }
        bytes
    }

    fn patch_has_ink(sheet: &SdfSheet, patch: [f32; 4]) -> bool {
        let [width, height] = sheet.atlas.size;
        let x0 = (patch[0].floor() as u32).min(width);
        let y0 = (patch[1].floor() as u32).min(height);
        let x1 = (patch[2].ceil() as u32).min(width);
        let y1 = (patch[3].ceil() as u32).min(height);
        (y0..y1).any(|y| {
            (x0..x1).any(|x| sheet.atlas.image[(y * width + x) as usize].is_sign_negative())
        })
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

    /// Exact regeneration command:
    ///
    /// `HARMONIGRAPH_REGENERATE_TEXT_SDF=1 cargo test --release -p harmonigraph-ui
    /// text_sdf::tests::regenerate_the_baked_sheet -- --ignored --exact`
    #[test]
    #[ignore = "writes the checked-in SDF asset"]
    fn regenerate_the_baked_sheet() {
        assert_eq!(
            std::env::var("HARMONIGRAPH_REGENERATE_TEXT_SDF").as_deref(),
            Ok("1"),
            "set HARMONIGRAPH_REGENERATE_TEXT_SDF=1 to confirm the source-tree write",
        );
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("text_sdf_sheet.bin");
        std::fs::write(&path, encode_sheet(&build_sheet()))
            .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
    }

    #[test]
    fn the_baked_sheet_is_byte_identical_to_exact_regeneration() {
        let start = std::time::Instant::now();
        let regenerated = encode_sheet(&build_sheet());
        eprintln!("exact text SDF regeneration: {:.3} ms", start.elapsed().as_secs_f64() * 1_000.0);
        assert_eq!(BAKED_SHEET, regenerated);
    }

    /// Reproducible cold-path measurement (the printed duration excludes the
    /// test harness):
    ///
    /// `cargo test --release -p harmonigraph-ui
    /// text_sdf::tests::the_baked_sheet_decodes_without_generating_fields --
    /// --exact --nocapture`
    #[test]
    fn the_baked_sheet_decodes_without_generating_fields() {
        let start = std::time::Instant::now();
        let decoded = decode_sheet(BAKED_SHEET).expect("the included sheet decodes");
        eprintln!("baked text SDF decode: {:.3} ms", start.elapsed().as_secs_f64() * 1_000.0);
        assert_eq!(
            decoded.atlas.image.len(),
            decoded.atlas.size.map(|side| side as usize).iter().product()
        );
    }

    #[test]
    fn every_shadowed_text_producer_character_and_drawn_mark_has_an_sdf() {
        let sheet = sheet();
        let mut produced = harmonigraph_core::NoteName::typeset_characters().to_vec();
        produced.extend(format!("{:.2}", -987.65).chars());
        for hz in [20.0, 50.0, 100.0, 200.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 10_000.0, 20_000.0] {
            produced.extend(crate::panes::spectral::frequency_label(hz).chars());
        }
        for db in [-100.0, -50.0, -20.0, 0.0] {
            produced.extend(crate::panes::spectral::level_label(db).chars());
        }
        produced.sort_unstable();
        produced.dedup();

        for ch in produced {
            assert!(sheet.type_patches.contains_key(&ch), "missing monospace `{ch}`");
        }
        let k = sheet.type_patches[&'k'];
        assert!(patch_has_ink(sheet, k.near), "the baked k has no near-field ink");
        assert!(patch_has_ink(sheet, k.coarse), "the baked k has no coarse-field ink");
        let mut packed: Vec<char> = sheet.type_patches.keys().copied().collect();
        packed.sort_unstable();
        let mut expected = shadowed_type_characters();
        expected.sort_unstable();
        assert_eq!(packed, expected, "the fixed sheet carries an unused glyph");
        for kind in MarkKind::ALL {
            let patch = sheet.mark_patches.get(&kind).unwrap_or_else(|| panic!("missing {kind:?}"));
            assert!(patch_has_ink(sheet, patch.near), "the baked {kind:?} has no near-field ink");
            assert!(
                patch_has_ink(sheet, patch.coarse),
                "the baked {kind:?} has no coarse-field ink"
            );
        }
    }
}
