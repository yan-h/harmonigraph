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
    // The layer count first: it says which layers the editor below shows.
    let mut layers = atmosphere.star_layers as f32;
    let top = harmonigraph_scene::star_plan::STAR_DEPTHS as f32;
    ValueBar::new(&mut layers, harmonigraph_scene::STAR_LAYERS_MIN as f32..=top, "Star layers")
        .integer()
        .show(ui)
        .on_hover_text(
            "How many depths of stars drift at their own speeds. They are drawn one over another, layer 1 at the back, and there is always a back and a front layer with the rest spaced evenly between. Which looks far is up to their values: smaller, denser, slower stars read as farther away. Each keeps its own size, spacing, speed, solid share and twinkle, set under Per layer, and a layer taken away keeps them for when it comes back. Fewer layers cost less.",
        );
    // Solo is held per depth, and a new layer count puts other depths under
    // the numbers, so a solo left on would come back on a different layer.
    if layers as u32 != atmosphere.star_layers {
        atmosphere.star_solo = Default::default();
    }
    atmosphere.star_layers = layers as u32;
    crate::widgets::star_layers(ui, atmosphere, size_scale);
    star_solo(ui, atmosphere);
    let drawn = harmonigraph_scene::star_plan::star_layer_depths(atmosphere.star_layers);
    let (back, front) =
        (drawn.iter().position(Option::is_some), drawn.iter().rposition(Option::is_some));
    let solid = [back, front].map(|k| atmosphere.star_solid[k.unwrap_or(0)]);
    crate::widgets::star_profile(ui, solid, &mut atmosphere.star_glow_falloff);
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
            "How irregularly stars are placed. 0% puts them at regular centers; 50% is half jitter; 100% is the original placement variation. Brightness and size have their own bars, Brightness variation and Size variation.",
        );
}

/// One toggle per drawn layer, numbered back to front as `Star layers` counts
/// them, between a `back` and a `front` caption. While any is on, only the
/// soloed layers are drawn.
fn star_solo(ui: &mut egui::Ui, stars: &mut harmonigraph_scene::StarSettings) {
    let layers = harmonigraph_scene::star_plan::star_layer_depths(stars.star_layers);
    ui.horizontal_wrapped(|ui| {
        crate::widgets::label(ui, "Solo");
        ui.spacing_mut().item_spacing.x = crate::theme::button_gap(crate::theme::ui_scale(ui.ctx()));
        let drawn = (0..layers.len()).filter(|&k| layers[k].is_some());
        crate::widgets::weak(ui, "back");
        for (n, k) in drawn.enumerate() {
            let name = (n + 1).to_string();
            ui.toggle_value(&mut stars.star_solo[k], crate::widgets::option_label(&name))
                .on_hover_text(format!(
                    "Draw only the soloed layers, to see what layer {name} looks like on its own (1 is drawn at the back). Not saved: every project opens with all layers drawn."
                ));
        }
        crate::widgets::weak(ui, "front");
    });
}

/// The resolution every star is drawn at, first among the Stars controls: a
/// quality and cost choice made once, where the rest shape the look.
pub(super) fn stars_resolution(ui: &mut egui::Ui, stars: &mut harmonigraph_scene::StarSettings) {
    ValueBar::new(
        &mut stars.star_resolution,
        harmonigraph_scene::STAR_RESOLUTION_MIN..=harmonigraph_scene::STAR_RESOLUTION_MAX,
        "Stars resolution",
    )
    .percent()
    .step(harmonigraph_scene::STAR_RESOLUTION_STEP)
    .show(ui)
    .on_hover_text(
        "Resolution the stars are drawn at, relative to the pane, in quarters. 100% is the sharpest; 50% draws a quarter of the pixels, softer and faster. Star positions, sizes and reach stay the same.",
    );
}

/// The stars' one motion control of their own beside `Drift direction`: each
/// layer's speed and twinkle are set under Per layer, among the look.
pub(super) fn stars_motion(ui: &mut egui::Ui, atmosphere: &mut harmonigraph_scene::StarSettings) {
    use harmonigraph_scene::{STAR_LIFETIME_MAX, STAR_LIFETIME_MIN};
    ValueBar::new(
        &mut atmosphere.star_lifetime,
        STAR_LIFETIME_MIN..=STAR_LIFETIME_MAX,
        "Star lifetime",
    )
    .eased(true)
    .unit(1.0, " s")
    .show(ui)
    .on_hover_text(
        "How long each star lives before a new one takes its place, alike at every layer. Twinkle, under Per layer, says how it gives way.",
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
