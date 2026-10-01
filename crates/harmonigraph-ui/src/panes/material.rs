//! Shared material controls; each pane supplies its own saved settings.
use crate::widgets::ValueBar;
use harmonigraph_scene::{SCALE_REFRACT_MAX, SCALE_REFRACT_MIN};

fn cloud_size_range() -> std::ops::RangeInclusive<f32> {
    harmonigraph_scene::CLOUD_SIZE_MIN..=harmonigraph_scene::CLOUD_SIZE_MAX
}

/// `pigment` shows the Edge pooling bar, which only the spectrogram draws.
pub(super) fn watercolor(
    ui: &mut egui::Ui,
    atmosphere: &mut harmonigraph_scene::MaterialSettings,
    pigment: bool,
) {
    ValueBar::new(&mut atmosphere.wash_size, cloud_size_range(), "Patch size")
        .eased(true)
        .unit(1.0, "\u{d7}")
        .show(ui)
        .on_hover_text(
            "Size of watercolor patches relative to the pane. 1× is about one twentieth of the pane's height; smaller values make finer grain and larger values make broader patches. The size follows the pane height.",
        );
    ValueBar::new(&mut atmosphere.wash_fuzz, 0.0..=1.0, "Edge feathering").percent().show(ui).on_hover_text(
        "Blend between neighboring watercolor patches. 0% makes hard-edged patches; 100% dissolves their edges. Does not change where each patch samples the picture.",
    );
    ValueBar::new(&mut atmosphere.wash_lobe, 0.0..=1.0, "Shape warp")
        .percent()
        .show(ui)
        .on_hover_text(
            "Distort round watercolor patches into lobes and streaks. 0% keeps them round; higher values stretch and bend their shapes.",
        );
    ValueBar::new(&mut atmosphere.wash_refract, 0.0..=1.0, "Refraction")
        .percent()
        .show(ui)
        .on_hover_text(
            "Pull the sampled picture toward each patch's center. 0% keeps the original picture; 100% gives each patch the level at its center.",
        );
    ValueBar::new(&mut atmosphere.wash_randomness, 0.0..=1.0, "Random brightness")
        .percent()
        .show(ui)
        .on_hover_text(
            "Vary brightness between watercolor patches with balanced brightening and dimming, preserving the average color. Highlights vary less to avoid clipping. 0% keeps the original brightness.",
        );
    ValueBar::new(&mut atmosphere.wash_layers, 0.0..=1.0, "Fine layer mix")
        .percent()
        .show(ui)
        .on_hover_text(
            "Mix a second layer of smaller watercolor patches over the broad layer. 0% uses the broad layer alone; 100% gives the fine layer its full strength.",
        );
    if pigment {
        ValueBar::new(&mut atmosphere.wash_pool, 0.0..=harmonigraph_scene::WASH_POOL_MAX, "Edge pooling")
            .percent()
            .show(ui)
            .on_hover_text(
                "Darken a patch in a soft crescent where another patch is painted over it, like pigment pooling at a dried edge. Only darkens, so silence stays black. Softer edges take less of it. 0% adds none; 50% is the strength Watercolor first shipped with, and 400% can pool to black at hard edges.",
            );
    }
}

pub(super) fn mosaic(ui: &mut egui::Ui, atmosphere: &mut harmonigraph_scene::MaterialSettings) {
    ValueBar::new(&mut atmosphere.scale_size, cloud_size_range(), "Cell size")
                        .eased(true)
                        .unit(1.0, "\u{d7}")
                        .show(ui)
                        .on_hover_text(
                            "Size of each mosaic cell relative to the pane. 1× is the reference size; larger values make broader cells. Refraction is a fraction of each cell's width, so larger cells also displace the picture farther.",
                        );
    ValueBar::new(&mut atmosphere.scale_variety, 0.0..=1.0, "Size variation")
                        .percent()
                        .show(ui)
                        .on_hover_text(
                            "Variation in mosaic cell size. 0% makes an even grid; 100% mixes small and large cells, with the largest about four times the smallest. The cells continue to cover the whole picture.",
                        );
    ValueBar::new(
                        &mut atmosphere.scale_refract,
                        SCALE_REFRACT_MIN..=SCALE_REFRACT_MAX,
                        "Refraction",
                    )
                    .unit(100.0, "%")
                    .show(ui)
                    .on_hover_text(
                        "Displacement of the picture within each mosaic cell, as a percentage of cell width. Positive values bend bands outward; negative values pull toward the center. -100% gives each cell one level; 0% leaves the picture unchanged.",
                    );
}

/// The starfield: pinpoints in depth drifting with parallax, and the one texture
/// that is light rather than a displaced reading of it.
///
/// Every quality that differed between the prototype's four motion variants is
/// a bar here rather than a choice made in the shader, because Yan's pick was a
/// starting point "with sliders exposed". The fresh values are his controls as
/// captured from the DAW on 2026-09-26.
/// `size_scale` is how many times the spectrogram's sizes this pane's run.
pub(super) fn stars(
    ui: &mut egui::Ui,
    atmosphere: &mut harmonigraph_scene::StarSettings,
    size_scale: f32,
) {
    use harmonigraph_scene::{
        StarSettings, STAR_DEFOCUS_MAX, STAR_DENSITY_MAX, STAR_DENSITY_MIN, STAR_FRINGE_MAX,
        STAR_SIZE_CURVE_MAX, STAR_SIZE_CURVE_MIN,
    };
    ValueBar::new(
        &mut atmosphere.star_density,
        STAR_DENSITY_MIN..=STAR_DENSITY_MAX,
        "Star density",
    )
    .unit(1.0, "\u{d7}")
    .show(ui)
    .on_hover_text("How many stars at every depth. Higher values pack them closer.");
    crate::widgets::depth(
        ui,
        &mut atmosphere.star_size_min,
        &mut atmosphere.star_size_max,
        &mut atmosphere.star_size_curve,
        StarSettings::size_range(size_scale),
        STAR_SIZE_CURVE_MIN..=STAR_SIZE_CURVE_MAX,
        true,
    );
    ValueBar::new(&mut atmosphere.star_randomness, 0.0..=1.0, "Brightness variation")
        .percent()
        .show(ui)
        .on_hover_text(
            "How much stars vary in brightness, keeping their average. Low values follow the underlying light evenly; high values make a few bright stars among many faint ones.",
        );
    ValueBar::new(&mut atmosphere.star_size_variation, 0.0..=1.0, "Size variation")
        .percent()
        .show(ui)
        .on_hover_text(
            "How much stars shrink below their depth's Star size, each at random. 0% makes every star at a depth the same size; 100% ranges down to about a tenth. Stars never grow past Star size.",
        );
    ValueBar::new(&mut atmosphere.star_jitter, 0.0..=1.0, "Position variation")
        .percent()
        .show(ui)
        .on_hover_text(
            "How irregularly stars are placed. 0% puts them at regular centers; 50% is half jitter; 100% is the original placement variation. Increasing Position variation also shortens distant halos, except in Uniform rendering. Brightness and size have their own bars, Brightness variation and Size variation.",
        );
    ValueBar::new(&mut atmosphere.star_far_fill, 0.0..=1.0, "Distant gap fill")
        .percent()
        .show(ui)
        .on_hover_text(
            "Fills thin background gaps between distant stars using their own colors. 0% keeps the original coverage, 50% fills gently, and 100% fills more strongly. Completely empty gaps remain empty.",
        );
    ValueBar::new(&mut atmosphere.star_fringe, 0.0..=STAR_FRINGE_MAX, "Halo strength")
        .percent()
        .show(ui)
        .on_hover_text(
            "A faint, wider fringe around every star in the star's own color. 0% draws bare soft points.",
        );
    ValueBar::new(&mut atmosphere.star_defocus, 0.0..=STAR_DEFOCUS_MAX, "Star softness")
        .percent()
        .show(ui)
        .on_hover_text("Widens every star's core and glow by the same proportion, at every depth. 0% keeps their base widths; Halo strength controls the strength of the surrounding glow.");
}

/// Always visible near the material choice, before motion and appearance controls.
pub(super) fn stars_quality(ui: &mut egui::Ui, stars: &mut harmonigraph_scene::StarSettings) {
    use harmonigraph_scene::StarHaloProfile;
    crate::widgets::choice_row(
        ui,
        "Stars rendering",
        &mut stars.star_halo_profile,
        &[
            (StarHaloProfile::Low, "Low", "Lower rendering cost with softer foreground points and coarser distant detail."),
            (StarHaloProfile::Medium, "Medium", "Faster rendering with softer stars, including the foreground."),
            (StarHaloProfile::P3, "High", "Sharper foreground stars with slightly softer distant stars and shorter distant glow."),
            (StarHaloProfile::Uniform, "Uniform", "Override halo resolution with one value for every depth."),
        ],
    );
    if stars.star_halo_profile == StarHaloProfile::Uniform {
        ValueBar::new(
            &mut stars.star_halo_resolution,
            harmonigraph_scene::STAR_HALO_RESOLUTION_MIN..=harmonigraph_scene::STAR_HALO_RESOLUTION_MAX,
            "Uniform halo resolution",
        )
        .percent()
        .show(ui)
        .on_hover_text(
            "Halo image width and height relative to the pane. 50% uses a quarter of the pixels; 100% uses native resolution. Lower values soften the glow. Star positions, sharp cores and halo reach stay the same.",
        );
    }
}

pub(super) fn stars_motion(ui: &mut egui::Ui, atmosphere: &mut harmonigraph_scene::StarSettings) {
    use harmonigraph_scene::{
        STAR_LIFETIME_MAX, STAR_LIFETIME_MIN, STAR_SPEED_CURVE_MAX, STAR_SPEED_CURVE_MIN,
        STAR_SPEED_MAX, STAR_SPEED_MIN,
    };
    crate::widgets::depth(
        ui,
        &mut atmosphere.star_speed_min,
        &mut atmosphere.star_speed_max,
        &mut atmosphere.star_speed_curve,
        STAR_SPEED_MIN..=STAR_SPEED_MAX,
        STAR_SPEED_CURVE_MIN..=STAR_SPEED_CURVE_MAX,
        false,
    );
    ValueBar::new(
        &mut atmosphere.star_lifetime,
        STAR_LIFETIME_MIN..=STAR_LIFETIME_MAX,
        "Star lifetime",
    )
    .eased(true)
    .unit(1.0, " s")
    .show(ui)
    .on_hover_text(
        "How long each star lives before a new one takes its place, fading in and out, alike at every depth.",
    );
}

/// The S1 body-light material; every body contributes its own sampled light.
pub(super) fn velvet(ui: &mut egui::Ui, s: &mut harmonigraph_scene::MaterialSettings) {
    ValueBar::new(&mut s.velvet_size, cloud_size_range(), "Cell size")
        .eased(true)
        .unit(1.0, "×")
        .show(ui)
        .on_hover_text("Size of each velvet scale. 1× is about six percent of the pane height.");
    ValueBar::new(&mut s.velvet_variety, 0.0..=1.0, "Size variation")
        .percent()
        .show(ui)
        .on_hover_text(
            "Variation in scale radius. 0% makes equal sizes; 50% is the Scales reference.",
        );
    ValueBar::new(&mut s.velvet_edge, 0.01..=1.0, "Edge softness")
        .percent()
        .show(ui)
        .on_hover_text("Width of the transitions between overlapping scales.");
    ValueBar::new(&mut s.velvet_irregularity, 0.0..=1.0, "Irregularity").percent().show(ui).on_hover_text("Move scale centers off their grid and smoothly warp their placement. Drift moves this fixed field in one direction.");
    ValueBar::new(&mut s.velvet_shape, 0.0..=1.0, "Scale shape")
        .percent()
        .show(ui)
        .on_hover_text("Round bodies at 0%; tapered overlapping scallops at 100%.");
}
