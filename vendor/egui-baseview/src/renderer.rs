#[cfg(all(feature = "opengl", not(feature = "wgpu")))]
mod opengl;
#[cfg(all(feature = "opengl", not(feature = "wgpu")))]
pub use opengl::renderer::{GraphicsConfig, Renderer};

#[cfg(feature = "wgpu")]
mod wgpu;
#[cfg(feature = "wgpu")]
pub use wgpu::renderer::{GraphicsConfig, Renderer, WgpuSetup};

/// The font atlas needs room for one frame's glyphs, not every texel the GPU
/// can address. egui makes its atlas this wide from its first allocation and
/// doubles the height as it fills, so an 8192 device limit leaves 64-128 MiB
/// CPU images behind after a zoom before the renderer and a full texture delta
/// hold their copies. At 4096, egui's 80% rebuild still clears accumulated
/// sizes before the atlas overflows.
const FONT_ATLAS_MAX_SIDE: usize = 4096;

fn font_atlas_max_texture_side(device_max: usize) -> usize {
    device_max.min(FONT_ATLAS_MAX_SIDE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_font_atlas_uses_the_device_limit_only_below_its_memory_bound() {
        assert_eq!(font_atlas_max_texture_side(2048), 2048);
        assert_eq!(font_atlas_max_texture_side(4096), 4096);
        assert_eq!(font_atlas_max_texture_side(8192), 4096);
    }
}
