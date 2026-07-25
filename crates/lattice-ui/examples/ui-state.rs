//! Print a UI-state blob (`SharedState::save_persist`) with chosen view and
//! camera fields overridden — the offline renderer's `--ui-state` input,
//! synthesized instead of recovered from a DAW project.
//!
//! `read-plugin-state.py` is the usual way to get one of these, but it needs
//! Bitwig, a saved project, and the editor window closed first. That is the
//! right tool for capturing a look Yan actually dialed in; it is the wrong one
//! for rendering a still of a look nobody has dialed in yet — a study frame,
//! a before/after for a settings change, a regression picture.
//!
//! ```text
//! cargo run -p lattice-ui --example ui-state -- \
//!     extent_sevens=1 sevens_size=0.55 cabinet_scale=0.71 > view.ron
//! lattice-offline chord.take --ui-state view.ron --layout lattice -o frame.png
//! ```
//!
//! Unknown keys are an error rather than a shrug: a typo that silently
//! rendered the default view would be indistinguishable from the setting
//! having no effect, which is exactly the question these frames are asked to
//! answer.

use lattice_render::wgpu::TextureFormat;
use lattice_ui::SharedState;

fn main() -> Result<(), String> {
    let mut state = SharedState::new(TextureFormat::Rgba8Unorm);

    for arg in std::env::args().skip(1) {
        let (key, value) = arg
            .split_once('=')
            .ok_or_else(|| format!("expected key=value, got `{arg}`"))?;
        let num = || -> Result<f32, String> {
            value.parse::<f32>().map_err(|_| format!("`{key}`: `{value}` is not a number"))
        };
        let int = || -> Result<i32, String> {
            value.parse::<i32>().map_err(|_| format!("`{key}`: `{value}` is not an integer"))
        };
        let flag = || -> Result<bool, String> {
            value.parse::<bool>().map_err(|_| format!("`{key}`: `{value}` is not true/false"))
        };
        let view = &mut state.view;
        match key {
            // Framing
            "extent_threes" => view.extent_threes = int()?,
            "extent_fives" => view.extent_fives = int()?,
            "extent_sevens" => view.extent_sevens = int()?,
            "center_threes" => view.center_threes = int()?,
            "center_fives" => view.center_fives = int()?,
            "center_sevens" => view.center_sevens = int()?,
            "spacing" => view.spacing = num()?,
            // The sevens layer's own look
            "sevens_size" => view.sevens_size = num()?,
            "sevens_gutter" => view.sevens_gutter = num()?,
            "sevens_label" => {
                view.sevens_label = ron::from_str(value)
                    .map_err(|e| format!("`sevens_label`: {e}"))?
            }
            // Node body
            "core_radius" => view.core_radius = num()?,
            "core_solidity" => view.core_solidity = num()?,
            "outer_inner" => view.outer_inner = num()?,
            "outer_outer" => view.outer_outer = num()?,
            "outer_gap" => view.outer_gap = num()?,
            "mark_thickness" => view.mark_thickness = num()?,
            "bloom_strength" => view.bloom_strength = num()?,
            "show_labels" => view.show_labels = flag()?,
            "show_cents" => view.show_cents = flag()?,
            "label_rim" => {
                view.label_rim = ron::from_str(value).map_err(|e| format!("`label_rim`: {e}"))?
            }
            "show_perf" => view.show_perf = flag()?,
            "frameless" => view.frameless = flag()?,
            "highlight_extremes" => {
                view.highlight_extremes = ron::from_str(value)
                    .map_err(|e| format!("`highlight_extremes`: {e}"))?
            }
            // Camera
            "distance" => state.camera.distance = num()?,
            "cabinet_scale" => state.camera.cabinet_scale = num()?,
            "cabinet_angle" => state.camera.cabinet_angle = num()?.to_radians(),
            "projection" => {
                state.camera.projection = ron::from_str(value)
                    .map_err(|e| format!("`projection`: {e}"))?
            }
            other => return Err(format!("unknown key `{other}`")),
        }
    }

    println!("{}", state.save_persist());
    Ok(())
}
