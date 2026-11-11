//! How both panes sample a displaced material: its cell on the pane and its
//! drift in those cells, decided here once and shipped to either shader.

/// Cloud space is the pane's, aspect-corrected and independent of DPI, and it
/// is FIXED: ten cloud units across the pane's height, which is what the
/// shipped `Cloud size` of 0.5x drew before the dial was retired. It was a
/// second copy of each texture's own size dial — the texture's size came out
/// as a product of the two — so pinning the frame here leaves each texture one
/// size dial that means its own size, and the drift to the dial named after it.
pub(crate) const CLOUD_UNITS: f32 = 10.0;
/// Watercolor's cells to a cloud unit at a `Patch size` of 1x. Set by the
/// picture and not by the cell count: a glob here was calibrated 1.18 cells
/// wide where the prototype's was `rad` 0.90, so matching J2's forty cells up
/// the pane would draw its globs 31% too big — measured against `j2.png`, a
/// visibly blobbier field with two or three fewer harmonic lines showing
/// through it. Matching the DIAMETER instead puts 40 * 1.18 / 0.90 ≈ 52 cells
/// up the pane at the fresh size.
pub(crate) const WASH_CELLS: f32 = 5.25;
/// Scales' cells to a cloud unit at a `Scale size` of 1: a cell of `24/405`
/// of the pane's height.
pub(crate) const VELVET_CELLS: f32 = 405.0 / 240.0;
/// Watercolor's tile period in cells: `PERIOD` in `lattice_material.wgsl`, and
/// the period of the spectrogram's production `CloudSampling`.
pub(crate) const WASH_PERIOD: f32 = 40.0;
/// Tile periods in one screen-axis repeat of Watercolor. The tile is turned by
/// the 3-4-5 rotation, (cos, sin) = (4/5, 3/5), before it is sampled, so the
/// first whole-number repeat along a screen axis is five periods.
pub(crate) const WASH_REPEAT_PERIODS: u32 = 5;

/// The size dial and cells per cloud unit at size 1, for either displaced material.
fn size_and_cells(velvet: bool, settings: &harmonigraph_scene::MaterialSettings) -> (f32, f32) {
    if velvet {
        (settings.velvet_size, VELVET_CELLS)
    } else {
        (settings.wash_size, WASH_CELLS)
    }
}

/// The material's cells to a cloud unit, which its travel is counted in.
pub(crate) fn cells_per_unit(velvet: bool, settings: &harmonigraph_scene::MaterialSettings) -> f64 {
    let (size, cells) = size_and_cells(velvet, settings);
    f64::from(cells / size)
}

/// One cell of the material on a pane `height` tall, in `height`'s own unit:
/// the pane's pixels for a texture or the lattice's target, its points for
/// the spectrogram's shader.
pub(crate) fn cell(
    velvet: bool,
    settings: &harmonigraph_scene::MaterialSettings,
    height: f32,
) -> f32 {
    let (size, cells) = size_and_cells(velvet, settings);
    height * size / (CLOUD_UNITS * cells)
}

/// The material's sampling offset in cells, as both shaders take it: the
/// `travel` ([`Motion::drift`](crate::travel::Motion::drift)), already in
/// cells, from the start, which the size scales as it scales the pane.
///
/// A screen-axis wrap of Watercolor is a whole-number repeat of its rotated
/// tile of `period` cells, so its offset is reduced onto one repeat before it
/// narrows to f32: unreduced, a long clock leaves the f32 fewer and fewer bits
/// of the cell it lands in. Scales' domain warp is nonperiodic: wrapping
/// would jump, so it passes unreduced, and from no start.
pub(crate) fn drift(
    velvet: bool,
    settings: &harmonigraph_scene::MaterialSettings,
    travel: [f64; 2],
    period: f32,
) -> [f32; 2] {
    let drift_period = f64::from(period) * f64::from(WASH_REPEAT_PERIODS);
    offset(velvet, settings, travel).map(|v| {
        if velvet {
            v as f32
        } else {
            v.rem_euclid(drift_period) as f32
        }
    })
}

/// [`drift`] before it is reduced and narrowed: the material's offset in its
/// own cells, still in f64, which runs on continuously where the drift jumps
/// a whole repeat. The spectrogram's colour history follows this.
pub(crate) fn offset(
    velvet: bool,
    settings: &harmonigraph_scene::MaterialSettings,
    travel: [f64; 2],
) -> [f64; 2] {
    if velvet {
        return travel;
    }
    let per_unit = cells_per_unit(velvet, settings);
    let start = harmonigraph_scene::MaterialSettings::DRIFT_START;
    std::array::from_fn(|axis| start[axis] * per_unit + travel[axis])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wash's drift reaches the shader reduced by whole repeats of its
    /// tile, so a long clock lands on the cell the f64 travel names — the
    /// 3-4-5 rotated basis included — within a ten-thousandth of a cell, where
    /// a plain cast to f32 misses it by cells at this clock.
    #[test]
    fn a_tiled_drift_is_reduced_by_whole_repeats_before_it_narrows() {
        let settings = harmonigraph_scene::MaterialSettings::default();
        let period = WASH_PERIOD;
        // Four months at the fresh speed.
        let rate = harmonigraph_scene::MaterialSettings::drift_rate(1.0, 37.0)
            .map(|v| v * cells_per_unit(false, &settings));
        let exact = rate.map(|v| v * 1.0e7);
        let start = harmonigraph_scene::MaterialSettings::DRIFT_START
            .map(|v| v * cells_per_unit(false, &settings));
        let exact = std::array::from_fn(|a| exact[a] + start[a]);
        // The tile coordinate the shader samples, in periods: a whole number
        // apart is the same texel.
        let uv = |r: [f64; 2]| {
            let r = r.map(|v| v / f64::from(period));
            [0.8 * r[0] + 0.6 * r[1], -0.6 * r[0] + 0.8 * r[1]]
        };
        let off = |a: [f64; 2], b: [f64; 2]| {
            let (a, b) = (uv(a), uv(b));
            (0..2).map(|i| (a[i] - b[i] - (a[i] - b[i]).round()).abs()).fold(0.0, f64::max)
                * f64::from(period)
        };
        let reduced = drift(false, &settings, rate.map(|v| v * 1.0e7), period).map(f64::from);
        let cast = exact.map(|v| f64::from(v as f32));
        assert!(off(reduced, exact) < 1e-4, "{} cells off", off(reduced, exact));
        assert!(off(cast, exact) > 0.01, "the clock is too short to need reducing");
    }
}
