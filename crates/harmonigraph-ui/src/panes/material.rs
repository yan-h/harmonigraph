//! Shared material controls; each pane supplies its own saved settings.
use crate::widgets::{RangeBar, ValueBar};
use harmonigraph_scene::{SCALE_REFRACT_MAX, SCALE_REFRACT_MIN};

fn cloud_size_range() -> std::ops::RangeInclusive<f32> {
    harmonigraph_scene::CLOUD_SIZE_MIN..=harmonigraph_scene::CLOUD_SIZE_MAX
}

pub(super) fn watercolor(ui: &mut egui::Ui, atmosphere: &mut harmonigraph_scene::MaterialSettings) {
    ValueBar::new(&mut atmosphere.wash_size, cloud_size_range(), "Glob size")
        .eased(true)
        .unit(1.0, "\u{d7}")
        .show(ui)
        .on_hover_text(
            "Size of watercolor globs relative to the pane. 1× is about one twentieth of the pane's height; smaller values make finer grain and larger values make broader patches. The size follows the pane height.",
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
            "Pull the sampled picture toward each glob's center. 0% keeps the original picture; 100% gives each glob the level at its center.",
        );
    ValueBar::new(&mut atmosphere.wash_randomness, 0.0..=1.0, "Random brightness")
        .percent()
        .show(ui)
        .on_hover_text(
            "Vary brightness between watercolor globs with balanced brightening and dimming, preserving the average color. Highlights vary less to avoid clipping. 0% keeps the original brightness.",
        );
    ValueBar::new(&mut atmosphere.wash_layers, 0.0..=1.0, "Fine layer mix")
        .percent()
        .show(ui)
        .on_hover_text(
            "Mix a second layer of smaller watercolor patches over the broad layer. 0% uses the broad layer alone; 100% gives the fine layer its full strength.",
        );
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

pub(super) fn speed(ui: &mut egui::Ui, value: &mut f32) {
    ValueBar::new(
        value,
        harmonigraph_scene::CLOUD_SPEED_MIN..=harmonigraph_scene::CLOUD_SPEED_MAX,
        "Drift speed",
    )
    .unit(1.0, "×")
    .show(ui)
    .on_hover_text(
        "1× carries the texture about a pane-height every four minutes. 0 holds it still.",
    );
}
pub(super) fn direction(ui: &mut egui::Ui, value: &mut f32) {
    ValueBar::new(
        value,
        harmonigraph_scene::CLOUD_DIRECTION_MIN..=harmonigraph_scene::CLOUD_DIRECTION_MAX,
        "Drift direction",
    )
    .integer()
    .unit(1.0, "°")
    .show(ui)
    .on_hover_text(
        "Constant direction of texture travel: 0° right, 90° down, 180° left and 270° up.",
    );
}

/// The starfield: pinpoints in depth drifting with parallax, and the one texture
/// that is light rather than a displaced reading of it.
///
/// Every quality that differed between the prototype's four motion variants is
/// a bar here rather than a choice made in the shader, because Yan's pick was a
/// starting point "with sliders exposed". The fresh values are that pick, V3, with round
/// 8's YB3 for how a star is coloured and shaped.
pub(super) fn stars(ui: &mut egui::Ui, atmosphere: &mut harmonigraph_scene::StarSettings) {
    use harmonigraph_scene::{
        STAR_DEFOCUS_MAX, STAR_DENSITY_MAX, STAR_DENSITY_MIN, STAR_FRINGE_MAX, STAR_LIFETIME_MAX,
        STAR_LIFETIME_MIN, STAR_SIZE_CURVE_MAX, STAR_SIZE_CURVE_MIN, STAR_SIZE_MAX, STAR_SIZE_MIN,
        STAR_SPEED_CURVE_MAX, STAR_SPEED_CURVE_MIN, STAR_SPEED_MAX, STAR_SPEED_MIN,
    };
    ValueBar::new(&mut atmosphere.star_density, STAR_DENSITY_MIN..=STAR_DENSITY_MAX, "Star density")
        .unit(1.0, "\u{d7}")
        .show(ui)
        .on_hover_text(
            "How many stars at every depth. Higher values pack them closer; past about 3\u{d7} the faintest dust is finer than a pixel and merges into texture.",
        );
    // Dragged in octaves, so the small end has room on the track: the sizes run
    // over two orders of magnitude and a linear track would crush 0.5 to 4
    // into its first few percent.
    let (mut small, mut big) = (atmosphere.star_size_min.log2(), atmosphere.star_size_max.log2());
    let response =
        RangeBar::new(&mut small, &mut big, STAR_SIZE_MIN.log2()..=STAR_SIZE_MAX.log2(), "Star size")
            .display(|octaves| format!("{:.1} px", octaves.exp2()))
            .show(ui)
            .on_hover_text(
                "The smallest and biggest stars: the farthest dust at the low end, the nearest stars at the high end, with Size curve deciding how the depths between share it out. Bigger stars are also farther apart.",
            );
    // Only on a change: the round trip through octaves is not exact, and
    // writing it back every frame would move the stored sizes by an ulp at a
    // time — and every star with them, since each depth's cells and its drift
    // in them key on them.
    if response.changed() {
        atmosphere.star_size_min = small.exp2();
        atmosphere.star_size_max = big.exp2();
    }
    ValueBar::new(
        &mut atmosphere.star_size_curve,
        STAR_SIZE_CURVE_MIN..=STAR_SIZE_CURVE_MAX,
        "Size curve",
    )
    .curve(|curve, p| p.powf(curve))
    .show(ui)
    .on_hover_text(
        "How the star spacing grows from the farthest depth to the nearest. 1 grows it evenly; higher values keep most depths fine dust and save the big stars for the nearest. The line previews it.",
    );
    ValueBar::new(&mut atmosphere.star_randomness, 0.0..=1.0, "Randomness")
        .percent()
        .show(ui)
        .on_hover_text(
            "How much stars vary in brightness and size. Low values follow the underlying light more evenly; high values make a few bright stars among many faint ones.",
        );
    ValueBar::new(&mut atmosphere.star_jitter, 0.0..=1.0, "Jitter")
        .percent()
        .show(ui)
        .on_hover_text(
            "How irregularly stars are placed. 0% puts them at regular centers; 50% is half jitter; 100% is the original placement variation. Increasing Jitter also shortens distant halos, except in Uniform rendering. Brightness and size variation are controlled by Randomness.",
        );
    use harmonigraph_scene::StarHaloProfile;
    crate::widgets::preset_row(
        ui,
        "Stars rendering",
        &["Medium", "High", "Uniform"],
        |ui, menu| {
            for (label, profile, hint) in [
            ("Medium", StarHaloProfile::Medium, "Faster rendering with softer stars, including the foreground."),
            ("High", StarHaloProfile::P3, "Sharper foreground stars with slightly softer distant stars and shorter distant glow."),
            ("Uniform", StarHaloProfile::Uniform, "Override halo resolution with one value for every depth."),
        ] {
            if ui.selectable_label(atmosphere.star_halo_profile == profile, label)
                .on_hover_text(hint).clicked()
            {
                atmosphere.star_halo_profile = profile;
                if menu {
                    ui.close();
                }
            }
        }
        },
    );
    if atmosphere.star_halo_profile == StarHaloProfile::Uniform {
        ValueBar::new(
            &mut atmosphere.star_halo_resolution,
            harmonigraph_scene::STAR_HALO_RESOLUTION_MIN..=harmonigraph_scene::STAR_HALO_RESOLUTION_MAX,
            "Uniform halo resolution",
        )
        .percent()
        .show(ui)
        .on_hover_text(
            "Halo image width and height relative to the pane. 50% uses a quarter of the pixels; 100% uses native resolution. Lower values soften the glow. Star positions, sharp cores and halo reach stay the same.",
        );
    }
    ValueBar::new(&mut atmosphere.star_far_fill, 0.0..=1.0, "Far fill")
        .percent()
        .show(ui)
        .on_hover_text(
            "Fills thin background gaps between distant stars using their own colors. 0% keeps the original coverage, 50% fills gently, and 100% fills more strongly. Completely empty gaps remain empty.",
        );
    ValueBar::new(&mut atmosphere.star_fringe, 0.0..=STAR_FRINGE_MAX, "Fringe")
        .percent()
        .show(ui)
        .on_hover_text(
            "A faint, wider fringe around every star in the star's own color. 0% draws bare soft points.",
        );
    RangeBar::new(
        &mut atmosphere.star_speed_min,
        &mut atmosphere.star_speed_max,
        STAR_SPEED_MIN..=STAR_SPEED_MAX,
        "Star speed",
    )
    .display(|speed| format!("{:.0}%", speed * 100.0))
    .show(ui)
    .on_hover_text(
        "How fast the farthest stars drift at the low end and the nearest at the high end, with Speed curve deciding how the depths between share it out. 100% carries a star a pane-height in about nine seconds; 0% holds it still. A wider range deepens the parallax; equal ends move every depth together.",
    );
    ValueBar::new(
        &mut atmosphere.star_speed_curve,
        STAR_SPEED_CURVE_MIN..=STAR_SPEED_CURVE_MAX,
        "Speed curve",
    )
    .curve(|curve, p| p.powf(curve))
    .show(ui)
    .on_hover_text(
        "How the drift speed grows from the farthest depth to the nearest. 1 steps it evenly; higher values keep most depths slow and the nearest fast, lower ones the reverse. The line previews it.",
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
    ValueBar::new(&mut atmosphere.star_defocus, 0.0..=STAR_DEFOCUS_MAX, "Star softness")
        .percent()
        .show(ui)
        .on_hover_text("Widens every star's core and glow by the same proportion, at every depth. 0% keeps their base widths; Fringe controls the strength of the surrounding glow.");
}
