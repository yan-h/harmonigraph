//! Shared material controls; each pane supplies its own saved settings.
use crate::widgets::ValueBar;

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
        STAR_CORE_MAX, STAR_CORE_MIN, STAR_DEPTH_CURVE_MAX, STAR_DEPTH_CURVE_MIN, STAR_FALLOFF_MAX,
        STAR_FALLOFF_MIN, STAR_GLOW_MAX, STAR_SIZE_MAX, STAR_SIZE_MIN, STAR_SPACING_MAX,
        STAR_SPACING_MIN,
    };
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
        &mut atmosphere.star_spacing_far,
        &mut atmosphere.star_spacing_near,
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
/// the plan the pane draws, at its `size_scale`.
fn held_to_fit(ui: &mut egui::Ui, stars: harmonigraph_scene::StarSettings, size_scale: f32) {
    let plan = stars.scaled(size_scale).plan();
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
    ValueBar::new(&mut s.velvet_square, 0.0..=1.0, "Squareness")
        .percent()
        .show(ui)
        .on_hover_text("Round scales at 0%; squares at 100%. For a tiled grid, also set Tilt, Irregularity and Scale shape to 0%.");
    ValueBar::new(&mut s.velvet_tilt, 0.0..=1.0, "Tilt").percent().show(ui).on_hover_text(
        "How far each scale turns off the pane's axes. 0% lines every scale up with the grid.",
    );
}
