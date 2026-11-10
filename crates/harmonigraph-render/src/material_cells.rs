//! Pane-height calibration shared by material travel and sampling.

pub(crate) const CLOUD_UNITS: f32 = 10.0;
pub(crate) const WASH_CELLS: f32 = 5.25;
/// Scales' cells to a cloud unit at a `Scale size` of 1: a cell of `24/405`
/// of the pane's height, `VELVET_CELL` in `spectrogram.wgsl`.
pub(crate) const VELVET_CELLS: f32 = 405.0 / 240.0;
/// Watercolor's tile period in cells: `PERIOD` in `lattice_material.wgsl`, and
/// the period of the spectrogram's production `CloudSampling`.
pub(crate) const WASH_PERIOD: f32 = 40.0;
/// Tile periods in one screen-axis repeat of Watercolor. The tile is turned by
/// the 3-4-5 rotation, (cos, sin) = (4/5, 3/5), before it is sampled, so the
/// first whole-number repeat along a screen axis is five periods.
pub(crate) const WASH_REPEAT_PERIODS: u32 = 5;

/// The size dial and cells per cloud unit at size 1, for either displaced material.
pub(crate) fn size_and_cells(
    velvet: bool,
    settings: &harmonigraph_scene::MaterialSettings,
) -> (f32, f32) {
    if velvet {
        (settings.velvet_size, VELVET_CELLS)
    } else {
        (settings.wash_size, WASH_CELLS)
    }
}

pub(crate) fn cells_per_unit(velvet: bool, settings: &harmonigraph_scene::MaterialSettings) -> f64 {
    let (size, cells) = size_and_cells(velvet, settings);
    f64::from(cells / size)
}
