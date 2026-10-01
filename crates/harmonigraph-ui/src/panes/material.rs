//! Shared material controls; each pane supplies its own saved settings.
use crate::widgets::ValueBar;
use harmonigraph_scene::{SCALE_REFRACT_MAX, SCALE_REFRACT_MIN};

fn cloud_size_range() -> std::ops::RangeInclusive<f32> {
    harmonigraph_scene::CLOUD_SIZE_MIN..=harmonigraph_scene::CLOUD_SIZE_MAX
}

pub(super) fn watercolor(ui: &mut egui::Ui, atmosphere: &mut harmonigraph_scene::MaterialSettings) {
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
}

/// Watercolor's tide line, which only the spectrogram draws.
pub(super) fn edge_pooling(
    ui: &mut egui::Ui,
    atmosphere: &mut harmonigraph_scene::SpectralAtmosphere,
) {
    let pool = harmonigraph_scene::WASH_POOL_MIN..=harmonigraph_scene::WASH_POOL_MAX;
    ValueBar::new(&mut atmosphere.wash_pool, pool, "Edge pooling")
        .percent()
        .show(ui)
        .on_hover_text(
            "Shade a patch along the edge of the patch painted over it, like pigment pooling at a dried edge. Above 0% it darkens; below 0% it lightens instead, like a bloom. Silence stays black either way. Softer edges take less of it. 50% is the strength Watercolor first shipped with, and 400% can pool to black at hard edges.",
        );
    let width = harmonigraph_scene::WASH_POOL_WIDTH_MIN..=harmonigraph_scene::WASH_POOL_WIDTH_MAX;
    ValueBar::new(&mut atmosphere.wash_pool_width, width, "Pooling width")
        .percent()
        .show(ui)
        .on_hover_text(
            "How far the edge shading reaches out from the edge, as a share of a patch's radius. Low values draw a thin line; high values a broad shadow. Does nothing while Edge pooling is 0%.",
        );
    ValueBar::new(&mut atmosphere.wash_pool_softness, 0.0..=1.0, "Pooling softness")
        .percent()
        .show(ui)
        .on_hover_text(
            "How the edge shading fades. 0% is a nearly even band with a hard outer edge; 100% is strongest at the edge with a long soft tail. 75% is the original crescent. Does nothing while Edge pooling is 0%.",
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

/// The starfield: pinpoints in depth drifting with parallax, and the one texture
/// that is light rather than a displaced reading of it.
///
/// Every quality that differed between the prototype's four motion variants is
/// a bar here rather than a choice made in the shader, because Yan's pick was a
/// starting point "with sliders exposed". The fresh values are his controls as
/// captured from the DAW on 2026-09-26.
/// `size_scale` is how many times its stored sizes and spacings this pane draws and shows.
pub(super) fn stars(
    ui: &mut egui::Ui,
    atmosphere: &mut harmonigraph_scene::StarSettings,
    size_scale: f32,
) {
    use crate::widgets::Depth;
    use harmonigraph_scene::{
        STAR_DEFOCUS_MAX, STAR_DENSITY_MAX, STAR_DENSITY_MIN, STAR_DEPTH_CURVE_MAX,
        STAR_DEPTH_CURVE_MIN, STAR_DIAMETER_MAX, STAR_DIAMETER_MIN, STAR_FRINGE_MAX,
        STAR_SPACING_MAX, STAR_SPACING_MIN,
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
        &mut atmosphere.star_diameter_min,
        &mut atmosphere.star_diameter_max,
        &mut atmosphere.star_diameter_curve,
        STAR_DIAMETER_MIN..=STAR_DIAMETER_MAX,
        STAR_DEPTH_CURVE_MIN..=STAR_DEPTH_CURVE_MAX,
        Depth::Size(size_scale),
    );
    crate::widgets::depth(
        ui,
        &mut atmosphere.star_spacing_min,
        &mut atmosphere.star_spacing_max,
        &mut atmosphere.star_spacing_curve,
        STAR_SPACING_MIN..=STAR_SPACING_MAX,
        STAR_DEPTH_CURVE_MIN..=STAR_DEPTH_CURVE_MAX,
        Depth::Spacing(size_scale),
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

/// Dev only: edits the [`StarPlan`](harmonigraph_scene::star_plan::StarPlan)
/// the renderer draws in place of the `Stars rendering` profile's, per depth
/// and per image, to measure what each choice costs and looks like. Nothing
/// here is saved; every load starts at production.
pub(super) fn stars_test_bed(ui: &mut egui::Ui, stars: &mut harmonigraph_scene::StarSettings) {
    use crate::widgets::choice_row;
    use harmonigraph_scene::star_plan::{
        StarGather, StarPlan, STAR_DEPTHS, STAR_FAR_DEPTHS, STAR_GAIN_MAX,
        STAR_IMAGE_RESOLUTION_MAX, STAR_IMAGE_RESOLUTION_MIN, STAR_PLAN_SCALE_MAX,
        STAR_PLAN_SCALE_MIN, STAR_REACH_MAX, STAR_REACH_MIN,
    };
    super::subsection(ui, "Star test bed (dev, not saved)", |ui| {
        let mut on = stars.test_bed.is_some();
        if crate::widgets::checkbox(ui, &mut on, "Draw from the test bed")
            .on_hover_text(
                "Draw the stars from the plan below instead of the Stars rendering profile. It starts from what the profile draws now. Not saved; every load starts at production.",
            )
            .changed()
        {
            stars.test_bed = on.then(|| StarPlan::production(*stars));
        }
        let fresh = StarPlan::production(*stars);
        let Some(plan) = stars.test_bed.as_mut() else {
            return;
        };
        if ui
            .button("Reset to production")
            .on_hover_text("Set every value below to what the Stars rendering profile draws now.")
            .clicked()
        {
            *plan = fresh;
        }
        let image = STAR_IMAGE_RESOLUTION_MIN..=STAR_IMAGE_RESOLUTION_MAX;
        ValueBar::new(&mut plan.far, image.clone(), "Far image").percent().show(ui).on_hover_text(
            "The far three depths are drawn into one image at this resolution and filtered up. 100% draws them at the pane's own resolution.",
        );
        ValueBar::new(&mut plan.near, image.clone(), "Near image").percent().show(ui).on_hover_text(
            "The near two depths are drawn over the far image at this resolution. 100% draws them straight into the pane, as does any value while Far image is at 100%.",
        );
        for (tier, name) in plan.halo_tiers.iter_mut().zip(["Halo A", "Halo B", "Halo C"]) {
            ValueBar::new(tier, image.clone(), name).percent().show(ui).on_hover_text(
                "Resolution of the halo image for every 3×3 depth that picks this tier.",
            );
        }
        let any_solo = plan.depths.iter().any(|depth| depth.solo);
        for (k, depth) in plan.depths.iter_mut().enumerate() {
            let image = if k < STAR_FAR_DEPTHS { "far image" } else { "near image" };
            let end = match k {
                0 => " · farthest",
                k if k == STAR_DEPTHS - 1 => " · nearest",
                _ => "",
            };
            let silenced = if any_solo && !depth.solo { " · silenced by solo" } else { "" };
            super::block(ui, &format!("Depth {} · {image}{end}{silenced}", k + 1));
            ui.push_id(k, |ui| {
                crate::widgets::checkbox(ui, &mut depth.solo, "Solo").on_hover_text(
                    "Draw only the soloed depths. The others count as Off, cost included, until no depth is soloed.",
                );
                choice_row(ui, "Gather", &mut depth.gather, &[
                    (StarGather::Off, "Off", "Not drawn or baked."),
                    (StarGather::Core, "1×1", "The whole star from one read, faded out where it would leave its own cell."),
                    (StarGather::Two, "2×2", "The whole star, glow included, from the four surrounding cells."),
                    (StarGather::Three, "3×3", "The core plus a halo image gathered from nine cells."),
                ]);
                ui.add_enabled_ui(depth.gather != StarGather::Off, |ui| {
                    let scale = STAR_PLAN_SCALE_MIN..=STAR_PLAN_SCALE_MAX;
                    ValueBar::new(&mut depth.scale, scale.clone(), "Scale")
                        .unit(1.0, "\u{d7}")
                        .show(ui)
                        .on_hover_text("Zooms this depth: its cells and its stars grow together, so it keeps its look with fewer, larger stars. Smaller means more stars and more cost.");
                    ValueBar::new(&mut depth.size, scale, "Size").unit(1.0, "\u{d7}").show(ui).on_hover_text(
                        "Star size relative to the scale. The cap at a third of the cell still applies, and 1×1 stars also stop at their cell's edge.",
                    );
                    ValueBar::new(&mut depth.gain, 0.0..=STAR_GAIN_MAX, "Opacity")
                        .unit(1.0, "\u{d7}")
                        .show(ui)
                        .on_hover_text("Multiplies every star's coverage at this depth.");
                    ValueBar::new(&mut depth.jitter, 0.0..=1.0, "Position variation")
                        .percent()
                        .show(ui)
                        .on_hover_text("Position variation for this depth alone. More variation shortens the reach the gather holds without seams.");
                    let window = match depth.gather {
                        StarGather::Three => Some("3×3"),
                        StarGather::Two => Some("2×2"),
                        StarGather::Off | StarGather::Core => None,
                    };
                    ui.add_enabled_ui(depth.gather != StarGather::Core, |ui| {
                        let bound = depth.gather.bound(depth.jitter);
                        ValueBar::new(&mut depth.reach, STAR_REACH_MIN..=STAR_REACH_MAX, "Glow reach")
                            .unit(1.0, " cells")
                            .show(ui)
                            .on_hover_text(match window {
                                Some(window) => format!(
                                    "How far the glow reaches, in cells. Fits up to {bound:.2} cells at this variation. Past that, the {window} read drops stars and the glow shows seams.",
                                ),
                                None => format!("1×1 reaches as far as its own cell allows: {bound:.2} cells at this variation."),
                            });
                    });
                    ui.add_enabled_ui(depth.gather == StarGather::Three, |ui| {
                        choice_row(ui, "Halo", &mut depth.tier, &[
                            (0, "A", "Drawn at Halo A's resolution."),
                            (1, "B", "Drawn at Halo B's resolution."),
                            (2, "C", "Drawn at Halo C's resolution."),
                        ]);
                    });
                });
            });
        }
    });
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
        crate::widgets::Depth::Speed,
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
