//! Shared material controls; each pane supplies its own saved settings.
use crate::widgets::ValueBar;
use harmonigraph_scene::{SCALE_REFRACT_MAX, SCALE_REFRACT_MIN};
use std::ops::RangeInclusive;

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
            "Shade a patch along the edge of the patch painted over it, like pigment pooling at a dried edge. Above 0% it darkens; below 0% it lightens instead, like a bloom. Silence stays on the gradient's floor either way. Softer edges take less of it. 50% is the strength Watercolor first shipped with, and 400% can pool to black at hard edges.",
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
        STAR_CORE_MAX, STAR_CORE_MIN, STAR_DENSITY_MAX, STAR_DENSITY_MIN, STAR_DEPTH_CURVE_MAX,
        STAR_DEPTH_CURVE_MIN, STAR_FALLOFF_MAX, STAR_FALLOFF_MIN, STAR_GLOW_MAX, STAR_SIZE_MAX,
        STAR_SIZE_MIN, STAR_SPACING_MAX, STAR_SPACING_MIN,
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
        STAR_SIZE_MIN..=STAR_SIZE_MAX,
        STAR_DEPTH_CURVE_MIN..=STAR_DEPTH_CURVE_MAX,
        Depth::Size(size_scale),
    );
    held_to_fit(ui, *atmosphere, size_scale);
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
            "How irregularly stars are placed. 0% puts them at regular centers; 50% is half jitter; 100% is the original placement variation. More variation leaves less room for big stars, so a depth may be drawn smaller to fit. Brightness and size have their own bars, Brightness variation and Size variation.",
        );
    ValueBar::new(&mut atmosphere.star_far_fill, 0.0..=1.0, "Distant gap fill")
        .percent()
        .show(ui)
        .on_hover_text(
            "Fills thin background gaps between distant stars using their own colors. 0% keeps the original coverage, 50% fills gently, and 100% fills more strongly. Completely empty gaps remain empty.",
        );
    ValueBar::new(&mut atmosphere.star_overlap_light, 0.0..=1.0, "Overlap light")
        .percent()
        .show(ui)
        .on_hover_text(
            "How overlapping stars at the same depth combine where together they cover the background completely. 0% averages their colors, so a cluster is no brighter than its average star; 100% adds their light, so a cluster grows brighter and paler as its colors fill. Stars that do not fully cover the background already add their light either way.",
        );
    ValueBar::new(&mut atmosphere.star_glow, 0.0..=STAR_GLOW_MAX, "Glow")
        .percent()
        .show(ui)
        .on_hover_text(
            "A soft glow around every star's core in the star's own color, fading out at the star's edge. This is its strength at the center; 0% draws bare cores, 100% a glow as bright as the core.",
        );
    for (core, label, end) in [
        (&mut atmosphere.star_core_far, "Core, far", "farthest"),
        (&mut atmosphere.star_core_near, "Core, near", "nearest"),
    ] {
        ValueBar::new(core, STAR_CORE_MIN..=STAR_CORE_MAX, label).percent().show(ui).on_hover_text(
            format!("How much of each of the {end} depth's stars is its bright core, as a share of the star's radius; the glow fills the rest. Depths between follow the Star size curve. Low values draw pinpoints in a wide glow; 100% spreads the core to the star's edge, a dense bed of soft stars."),
        );
    }
    ValueBar::new(&mut atmosphere.star_falloff, STAR_FALLOFF_MIN..=STAR_FALLOFF_MAX, "Glow falloff")
        .show(ui)
        .on_hover_text(
            "How quickly the glow fades toward the star's edge. Low values spread it as a broad haze reaching the edge; high values draw it in as a tight bloom around the core.",
        );
}

/// A muted line under `Star size` while any depth's stars are drawn smaller
/// than the dials ask, to fit the widest read their spacing allows. It reads
/// the plan the pane draws, at its `size_scale`, without the dev test bed.
fn held_to_fit(ui: &mut egui::Ui, stars: harmonigraph_scene::StarSettings, size_scale: f32) {
    let plan =
        harmonigraph_scene::StarSettings { test_bed: None, ..stars.scaled(size_scale) }.plan();
    let held: Vec<_> =
        plan.depths.iter().enumerate().filter(|(_, depth)| depth.clamped()).collect();
    let Some(widest) = held.iter().map(|(_, depth)| 2.0 * depth.radius).reduce(f32::max) else {
        return;
    };
    let names = held.iter().map(|(k, _)| (k + 1).to_string()).collect::<Vec<_>>().join(", ");
    let (depths, their) = if held.len() == 1 { ("Depth", "its") } else { ("Depths", "their") };
    crate::widgets::weak(
        ui,
        format!("{depths} {names} drawn smaller to fit {their} spacing, at most {widest:.1} px."),
    );
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
            (StarHaloProfile::P3, "High", "Sharper foreground stars with slightly softer distant stars."),
            (StarHaloProfile::Uniform, "Uniform", "Draw at the pane's resolution, with one halo resolution for every depth that needs a halo."),
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

/// Dev only: overrides on the [`StarPlan`](harmonigraph_scene::star_plan::StarPlan)
/// the settings give, per depth and per image, to measure what each choice
/// costs and looks like. Every row shows what the renderer draws, at the
/// pane's `size_scale`, and an edit there becomes an override. Nothing here is
/// saved; every load starts at production.
pub(super) fn stars_test_bed(
    ui: &mut egui::Ui,
    stars: &mut harmonigraph_scene::StarSettings,
    size_scale: f32,
) {
    use crate::widgets::choice_row;
    use harmonigraph_scene::star_plan::{
        StarGather, StarTestBed, STAR_DEPTHS, STAR_FAR_DEPTHS, STAR_GAIN_MAX,
        STAR_IMAGE_RESOLUTION_MAX, STAR_IMAGE_RESOLUTION_MIN, STAR_PLAN_SCALE_MAX,
        STAR_PLAN_SCALE_MIN,
    };
    use harmonigraph_scene::{
        STAR_CORE_MAX, STAR_CORE_MIN, STAR_FALLOFF_MAX, STAR_FALLOFF_MIN, STAR_GLOW_MAX,
    };
    let window = |gather: StarGather| match gather {
        StarGather::Off => "Off",
        StarGather::Core => "1×1",
        StarGather::Two => "2×2",
        StarGather::Three => "3×3",
    };
    super::subsection(ui, "Star test bed (dev, not saved)", |ui| {
        let mut on = stars.test_bed.is_some();
        if crate::widgets::checkbox(ui, &mut on, "Draw from the test bed")
            .on_hover_text(
                "Apply the overrides below on top of what the settings draw. Every row shows the value drawn now; editing one overrides it. Not saved; every load starts at production.",
            )
            .changed()
        {
            stars.test_bed = on.then(StarTestBed::default);
        }
        // What the renderer draws, the bed included, in the pane's star pixels.
        let plan = stars.scaled(size_scale).plan();
        let Some(bed) = stars.test_bed.as_mut() else {
            return;
        };
        if ui
            .button("Reset all")
            .on_hover_text("Follow the settings again on every row below.")
            .clicked()
        {
            *bed = StarTestBed::default();
        }
        let image = STAR_IMAGE_RESOLUTION_MIN..=STAR_IMAGE_RESOLUTION_MAX;
        override_bar(ui, &mut bed.far, plan.far, image.clone(), "Far image", Unit::Percent).on_hover_text(
            "The far three depths are drawn into one image at this resolution and filtered up. 100% draws them at the pane's own resolution.",
        );
        override_bar(ui, &mut bed.near, plan.near, image.clone(), "Near image", Unit::Percent).on_hover_text(
            "The near two depths are drawn over the far image at this resolution. 100% draws them straight into the pane, as does any value while Far image is at 100%.",
        );
        for ((tier, live), name) in
            bed.halo_tiers.iter_mut().zip(plan.halo_tiers).zip(["Halo A", "Halo B", "Halo C"])
        {
            override_bar(ui, tier, live, image.clone(), name, Unit::Percent).on_hover_text(
                "Resolution of the halo image for every 3×3 depth that picks this tier.",
            );
        }
        let any_solo = bed.depths.iter().any(|depth| depth.solo);
        for (k, (depth, drawn)) in bed.depths.iter_mut().zip(plan.depths).enumerate() {
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
                    (None, "Auto", "The cheapest read that holds the stars whole, or 3×3 with the stars drawn smaller to fit."),
                    (Some(StarGather::Off), "Off", "Not drawn or baked."),
                    (Some(StarGather::Core), "1×1", "The whole star from its own cell, held inside it."),
                    (Some(StarGather::Two), "2×2", "The whole star, glow included, from the four surrounding cells."),
                    (Some(StarGather::Three), "3×3", "The star's part inside its own cell, plus a halo image gathered from nine cells."),
                ]);
                ui.add_enabled_ui(drawn.gather != StarGather::Off, |ui| {
                    let read = match depth.gather {
                        None => format!("Auto → {}", window(drawn.gather)),
                        Some(gather) => window(gather).to_owned(),
                    };
                    let held = if drawn.clamped() {
                        format!(" (asked {:.1}, held to fit {})", drawn.wanted, window(drawn.gather))
                    } else {
                        String::new()
                    };
                    crate::widgets::weak(ui, format!("{read} · radius {:.1} px{held}", drawn.radius));
                    let scale = STAR_PLAN_SCALE_MIN..=STAR_PLAN_SCALE_MAX;
                    let times = Unit::Times;
                    multiplier_bar(ui, &mut depth.scale, scale.clone(), "Scale", times).on_hover_text(
                        "Zooms this depth: its cells and its stars grow together, so it keeps its look with fewer, larger stars. Smaller means more stars and more cost.",
                    );
                    multiplier_bar(ui, &mut depth.size, scale, "Size", times).on_hover_text(
                        "Star size relative to the scale. Auto picks a wider read for bigger stars; past 3×3, or the gather picked here, they are drawn smaller to fit.",
                    );
                    multiplier_bar(ui, &mut depth.gain, 0.0..=STAR_GAIN_MAX, "Opacity", times)
                        .on_hover_text("Multiplies every star's coverage at this depth.");
                    override_bar(ui, &mut depth.jitter, drawn.jitter, 0.0..=1.0, "Position variation", Unit::Percent)
                        .on_hover_text("Position variation for this depth alone. More variation leaves less room in each read, so stars may be drawn smaller or need a wider read.");
                    override_bar(ui, &mut depth.core, drawn.core, STAR_CORE_MIN..=STAR_CORE_MAX, "Core", Unit::Percent)
                        .on_hover_text("Core for this depth alone: the share of the star's radius that is its bright core.");
                    override_bar(ui, &mut depth.glow, drawn.glow, 0.0..=STAR_GLOW_MAX, "Glow", Unit::Percent)
                        .on_hover_text("Glow for this depth alone: its strength at the star's center.");
                    override_bar(ui, &mut depth.falloff, drawn.falloff, STAR_FALLOFF_MIN..=STAR_FALLOFF_MAX, "Glow falloff", Unit::Plain)
                        .on_hover_text("Glow falloff for this depth alone: low spreads the glow as a broad haze, high draws it in as a tight bloom.");
                    ui.add_enabled_ui(drawn.gather == StarGather::Three, |ui| {
                        follow_row(ui, &mut depth.tier, None, |ui| {
                            let mut tier = drawn.tier;
                            choice_row(ui, "Halo", &mut tier, &[
                                (0, "A", "Drawn at Halo A's resolution."),
                                (1, "B", "Drawn at Halo B's resolution."),
                                (2, "C", "Drawn at Halo C's resolution."),
                            ]);
                            ((tier != drawn.tier).then_some(Some(tier)), ())
                        });
                    });
                });
            });
        }
    });
}

/// How a test bed bar reads out.
#[derive(Clone, Copy)]
enum Unit {
    Percent,
    Times,
    Plain,
}

/// One test bed row with a follow control ahead of it: `row` draws the row
/// and returns the value to set, if it was edited, and ↺, shown while `value`
/// is not `follow`, puts it back. The control keeps its place while hidden, so
/// a bar does not shift under the pointer as an edit starts.
fn follow_row<T: PartialEq, R>(
    ui: &mut egui::Ui,
    value: &mut T,
    follow: T,
    row: impl FnOnce(&mut egui::Ui) -> (Option<T>, R),
) -> R {
    ui.horizontal(|ui| {
        let side = crate::theme::row_height(crate::theme::ui_scale(ui.ctx()));
        let reset = egui::Button::new("\u{21ba}").min_size(egui::Vec2::splat(side));
        let followed = ui
            .add_visible(*value != follow, reset)
            .on_hover_text("Follow the settings again")
            .clicked();
        let (edited, out) = row(ui);
        if let Some(edited) = edited {
            *value = edited;
        }
        if followed {
            *value = follow;
        }
        out
    })
    .inner
}

fn unit_bar<'a>(
    value: &'a mut f32,
    range: RangeInclusive<f32>,
    label: &'a str,
    unit: Unit,
) -> ValueBar<'a> {
    let bar = ValueBar::new(value, range, label);
    match unit {
        Unit::Percent => bar.percent(),
        Unit::Times => bar.unit(1.0, "\u{d7}"),
        Unit::Plain => bar,
    }
}

/// A bar over an override that follows the settings while `None`: it shows
/// `live`, what the renderer draws, and an edit sets the override.
fn override_bar(
    ui: &mut egui::Ui,
    over: &mut Option<f32>,
    live: f32,
    range: RangeInclusive<f32>,
    label: &str,
    unit: Unit,
) -> egui::Response {
    follow_row(ui, over, None, |ui| {
        let mut value = live;
        let response = unit_bar(&mut value, range, label, unit).show(ui);
        (response.changed().then_some(Some(value)), response)
    })
}

/// A bar over a multiplier that follows the settings at 1.
fn multiplier_bar(
    ui: &mut egui::Ui,
    times: &mut f32,
    range: RangeInclusive<f32>,
    label: &str,
    unit: Unit,
) -> egui::Response {
    let mut value = *times;
    follow_row(ui, times, 1.0, |ui| {
        let response = unit_bar(&mut value, range, label, unit).show(ui);
        (response.changed().then_some(value), response)
    })
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
