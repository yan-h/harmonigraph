//! Pane-height calibration shared by material travel and sampling.

pub(crate) const CLOUD_UNITS: f32 = 10.0;
pub(crate) const WASH_CELLS: f32 = 5.25;

/// The size dial and cells per cloud unit at size 1, for either displaced material.
pub(crate) fn size_and_cells(
    velvet: bool,
    settings: &harmonigraph_scene::MaterialSettings,
) -> (f32, f32) {
    if velvet {
        (settings.velvet_size, 405.0 / 240.0)
    } else {
        (settings.wash_size, WASH_CELLS)
    }
}

pub(crate) fn cells_per_unit(velvet: bool, settings: &harmonigraph_scene::MaterialSettings) -> f64 {
    let (size, cells) = size_and_cells(velvet, settings);
    f64::from(cells / size)
}
