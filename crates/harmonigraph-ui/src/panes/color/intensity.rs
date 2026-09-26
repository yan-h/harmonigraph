//! Target-owned controls and possible-reach previews for note mappings.
//! No live-note selection or cached derived state: ranges use the same sum
//! as the picture, and are painted after this frame's controls have edited it.

use egui::{Color32, Pos2, Rect, Stroke, Ui, Vec2};
use harmonigraph_scene::{
    IntensityReach, IntensitySettings, IntensitySource, IntensityTarget, BLOOM_MAX,
    INTENSITY_WEIGHT_MAX, THICKNESS_MAX_RANGE,
};

use crate::{theme, widgets};
use widgets::ValueBar;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Velocity,
    Pressure,
    Timbre,
    Gain,
}

impl Source {
    const ALL: [Self; 4] = [Self::Velocity, Self::Pressure, Self::Timbre, Self::Gain];

    fn name(self) -> &'static str {
        match self {
            Self::Velocity => "Velocity",
            Self::Pressure => "Pressure",
            Self::Timbre => "Timbre",
            Self::Gain => "Gain",
        }
    }

    fn setting(self, settings: &mut IntensitySettings) -> &mut IntensitySource {
        match self {
            Self::Velocity => &mut settings.velocity,
            Self::Pressure => &mut settings.pressure,
            Self::Timbre => &mut settings.timbre,
            Self::Gain => &mut settings.gain,
        }
    }

    fn color(self) -> Color32 {
        // Vivid source identities; fills mix these with the current skin's well.
        match self {
            Self::Velocity => Color32::from_rgb(46, 164, 255),
            Self::Pressure => Color32::from_rgb(255, 139, 25),
            Self::Timbre => Color32::from_rgb(174, 76, 255),
            Self::Gain => Color32::from_rgb(30, 200, 102),
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Velocity => "Velocity adds 0 to one whole weight above the base.",
            Self::Pressure => "Pressure adds 0 to one whole weight above the base.",
            Self::Timbre => "Minimum timbre subtracts the whole weight; center adds nothing; maximum adds the whole weight.",
            Self::Gain => "Silence adds nothing; unity gain adds the whole weight. Higher gain extends the dashed part of the indicator.",
        }
    }

    fn reach(self, settings: &IntensitySettings, target: IntensityTarget) -> IntensityReach {
        let mut single = *settings;
        for source in Self::ALL {
            if source != self {
                *source.setting(&mut single).weight_mut(target) = None;
            }
        }
        single.reach(target)
    }
}

fn target_name(target: IntensityTarget) -> &'static str {
    match target {
        IntensityTarget::Opacity => "Opacity",
        IntensityTarget::Glow => "Bloom",
        IntensityTarget::Thickness => "Thickness",
    }
}

fn limits(settings: &IntensitySettings, target: IntensityTarget) -> (f32, f32) {
    match target {
        IntensityTarget::Opacity => (settings.opacity_rest, 1.0),
        IntensityTarget::Glow => (settings.glow_base, BLOOM_MAX),
        IntensityTarget::Thickness => (settings.thickness_base, settings.thickness_max),
    }
}

pub(super) fn show(ui: &mut Ui, settings: &mut IntensitySettings) {
    widgets::weak(ui, "Bands show possible reach. Stripes mark clipping; dashes mark gain boosts.");
    // Apply additions/removals after drawing so rows keep stable geometry this frame.
    let mut route = None;
    for target in [IntensityTarget::Opacity, IntensityTarget::Thickness, IntensityTarget::Glow] {
        ui.push_id(target_name(target), |ui| {
            group(ui, settings, target, &mut route);
        });
    }
    if let Some((source, target, weight)) = route {
        *source.setting(settings).weight_mut(target) = weight;
        ui.ctx().request_repaint();
    }
}

fn group(
    ui: &mut Ui,
    settings: &mut IntensitySettings,
    target: IntensityTarget,
    route: &mut Option<(Source, IntensityTarget, Option<f32>)>,
) {
    let scale = theme::ui_scale(ui.ctx());
    ui.horizontal_wrapped(|ui| {
        widgets::label(ui, target_name(target));
        egui::containers::menu::MenuButton::new("Add mapping")
            .config(egui::containers::menu::MenuConfig::new().style(widgets::menu_style(ui.ctx())))
            .ui(ui, |ui| {
                for source in Source::ALL {
                    let enabled = source.setting(settings).weight(target).is_some();
                    if ui.add_enabled(!enabled, egui::Button::new(source.name())).clicked() {
                        *route = Some((source, target, Some(1.0)));
                        ui.close();
                    }
                }
            });
    });
    if target == IntensityTarget::Thickness {
        ValueBar::new(&mut settings.thickness_max, THICKNESS_MAX_RANGE, "Thickness max")
            .unit(1.0, "×")
            .show(ui)
            .on_hover_text("The thickness ceiling, in multiples of each pane's reference width. Lowering it also limits Thickness base.");
        settings.thickness_base = settings.thickness_base.min(settings.thickness_max);
    }
    let sources: Vec<_> = Source::ALL
        .into_iter()
        .filter(|source| source.setting(settings).weight(target).is_some())
        .collect();
    let mut overlay = None;
    let base = match target {
        IntensityTarget::Opacity => ValueBar::new(&mut settings.opacity_rest, 0.0..=1.0, "Opacity base")
            .overlay_slot(&mut overlay).show(ui).on_hover_text("Starting opacity, even with no mappings. Only timbre can reduce it. A base of 1 leaves no room for positive additions."),
        IntensityTarget::Glow => ValueBar::new(&mut settings.glow_base, 0.0..=BLOOM_MAX, "Bloom base")
            .overlay_slot(&mut overlay).unit(1.0, "×").show(ui).on_hover_text("Starting note bloom, even with no mappings. Mappings can add bloom from base 0. The separate lattice background glow is unchanged."),
        IntensityTarget::Thickness => ValueBar::new(&mut settings.thickness_base, 0.0..=settings.thickness_max, "Thickness base")
            .overlay_slot(&mut overlay).unit(1.0, "×").show(ui).on_hover_text("Starting thickness. 1× is Ribbon width in the Analyzer and the MIDI layer width in the Lattice. The mappings add multiples of those same reference widths; a hidden layer remains hidden."),
    };
    for &source in &sources {
        ui.push_id(source.name(), |ui| {
            ui.horizontal(|ui| {
                let delete_width = 52.0 * scale;
                let width =
                    (ui.available_width() - delete_width - ui.spacing().item_spacing.x).max(0.0);
                let label = format!("{} weight", source.name());
                ui.allocate_ui_with_layout(
                    Vec2::new(width, theme::row_height(scale)),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        let mut bar = ValueBar::new(
                            source.setting(settings).weight_mut(target).as_mut().unwrap(),
                            0.0..=INTENSITY_WEIGHT_MAX,
                            &label,
                        )
                        .color(source.color());
                        if source == Source::Timbre {
                            bar = bar.display(|v| format!("±{v:.2}"));
                        }
                        bar.show(ui).on_hover_text(source.hint())
                    },
                );
                if ui
                    .add_sized(
                        Vec2::new(delete_width, theme::row_height(scale)),
                        egui::Button::new("Delete"),
                    )
                    .on_hover_text(format!("Remove {} from {}", source.name(), target_name(target)))
                    .clicked()
                {
                    *route = Some((source, target, None));
                }
            });
        });
    }
    if let Some(slot) = overlay.filter(|_| !sources.is_empty()) {
        let rect = base.rect.shrink(2.0 * scale);
        ui.painter()
            .set(slot, egui::Shape::Vec(reach_shapes(rect, settings, target, &sources, scale)));
    } else if sources.is_empty() {
        widgets::weak(ui, "No mappings");
    }
}

fn band_color(source: Source) -> Color32 {
    theme::well().lerp_to_gamma(source.color(), 0.65)
}

fn reach_shapes(
    rect: Rect,
    settings: &IntensitySettings,
    target: IntensityTarget,
    sources: &[Source],
    scale: f32,
) -> Vec<egui::Shape> {
    let (_, ceiling) = limits(settings, target);
    let mut shapes = Vec::new();
    let height = rect.height() / sources.len() as f32;
    for (i, &source) in sources.iter().enumerate() {
        let lane = Rect::from_min_size(
            rect.min + Vec2::new(0.0, i as f32 * height),
            Vec2::new(rect.width(), height),
        );
        band_shapes(
            &mut shapes,
            lane,
            source.reach(settings, target),
            ceiling,
            band_color(source),
            scale,
        );
    }
    shapes
}

fn band_shapes(
    shapes: &mut Vec<egui::Shape>,
    rect: Rect,
    reach: IntensityReach,
    ceiling: f32,
    color: Color32,
    scale: f32,
) {
    let x = |value: f32| rect.left() + rect.width() * (value / ceiling).clamp(0.0, 1.0);
    let span = Rect::from_min_max(
        Pos2::new(x(reach.min), rect.top()),
        Pos2::new(x(reach.max), rect.bottom()),
    );
    let corners = egui::CornerRadius {
        nw: 0,
        sw: 0,
        ne: theme::control_radius(scale),
        se: theme::control_radius(scale),
    };
    if span.width() > 0.0 {
        shapes.push(egui::Shape::rect_filled(span, corners, color));
    }
    if reach.gain_boosts {
        let mut start = x(reach.max);
        while start < rect.right() {
            shapes.push(egui::Shape::line_segment(
                [
                    Pos2::new(start, rect.center().y),
                    Pos2::new((start + 3.0 * scale).min(rect.right()), rect.center().y),
                ],
                Stroke::new(scale, color),
            ));
            start += 6.0 * scale;
        }
    }
    for (clipped, at) in
        [(reach.min < 0.0, rect.left()), (reach.max > ceiling, rect.right() - 5.0 * scale)]
    {
        if clipped {
            let end = Rect::from_min_size(
                Pos2::new(at, rect.top()),
                Vec2::new(5.0 * scale, rect.height()),
            )
            .intersect(rect);
            shapes.push(egui::Shape::rect_filled(end, corners, color));
            // Bound each diagonal explicitly: the reserved shape shares the
            // slider's painter, so there is no per-lane clip rectangle.
            let mut x = end.left() - end.height();
            while x < end.right() {
                let low = (end.left() - x).max(0.0);
                let high = (end.right() - x).min(end.height());
                if low < high {
                    shapes.push(egui::Shape::line_segment(
                        [
                            Pos2::new(x + low, end.bottom() - low),
                            Pos2::new(x + high, end.bottom() - high),
                        ],
                        Stroke::new(scale, theme::well()),
                    ));
                }
                x += 3.0 * scale;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(
        ctx: &egui::Context,
        settings: &mut IntensitySettings,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(280.0, 1100.0))),
                events,
                ..Default::default()
            },
            |ui| show(ui, settings),
        )
    }

    fn texts(output: &egui::FullOutput, text: &str) -> Vec<Rect> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.galley.text() == text => {
                    Some(Rect::from_min_size(t.pos, t.galley.size()))
                }
                _ => None,
            })
            .collect()
    }

    fn click(ctx: &egui::Context, settings: &mut IntensitySettings, pos: Pos2) -> egui::FullOutput {
        frame(ctx, settings, vec![egui::Event::PointerMoved(pos)]);
        frame(
            ctx,
            settings,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(
            ctx,
            settings,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        )
    }

    #[test]
    fn target_menus_add_independent_weights_and_delete_only_that_mapping() {
        let ctx = crate::tests::probe::themed();
        let mut settings = IntensitySettings {
            velocity: IntensitySource { opacity: Some(0.4), ..Default::default() },
            ..Default::default()
        };
        let out = frame(&ctx, &mut settings, vec![]);
        let add = texts(&out, "Add mapping");
        assert_eq!(add.len(), 3);
        click(&ctx, &mut settings, add[1].center());
        let menu = frame(&ctx, &mut settings, vec![]);
        let item = texts(&menu, "Velocity")[0];
        click(&ctx, &mut settings, item.center());
        assert_eq!(settings.velocity.opacity, Some(0.4));
        assert_eq!(settings.velocity.thickness, Some(1.0));
        let out = frame(&ctx, &mut settings, vec![]);
        assert_eq!(texts(&out, "Velocity weight").len(), 2);
        let remove = texts(&out, "Delete");
        assert_eq!(remove.len(), 2);
        click(&ctx, &mut settings, remove[0].center());
        assert_eq!(settings.velocity.opacity, None);
        assert_eq!(settings.velocity.thickness, Some(1.0));
        let out = frame(&ctx, &mut settings, vec![]);
        assert_eq!(texts(&out, "Velocity weight").len(), 1);
    }

    #[test]
    fn inset_bands_stay_under_text_and_follow_base_drags() {
        let ctx = crate::tests::probe::themed();
        let mut settings = IntensitySettings {
            opacity_rest: 0.25,
            velocity: IntensitySource { opacity: Some(0.5), ..Default::default() },
            pressure: IntensitySource { opacity: Some(0.25), ..Default::default() },
            timbre: IntensitySource { opacity: Some(0.5), ..Default::default() },
            gain: IntensitySource { opacity: Some(1.25), ..Default::default() },
            ..Default::default()
        };
        let out = frame(&ctx, &mut settings, vec![]);
        let base_name = texts(&out, "Opacity base")[0];
        let base = out
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Rect(r)
                    if r.fill == theme::well() && r.rect.contains(base_name.center()) =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .unwrap();
        let plot = base.shrink(2.0);
        fn overlay(out: &egui::FullOutput) -> &[egui::Shape] {
            out.shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Vec(shapes) => Some(shapes.as_slice()),
                    _ => None,
                })
                .unwrap()
        }
        let band = |out: &egui::FullOutput, color: Color32| {
            overlay(out)
                .iter()
                .find_map(|s| match s {
                    egui::Shape::Rect(r) if r.fill == color && r.rect.width() > 10.0 => {
                        Some(r.rect)
                    }
                    _ => None,
                })
                .unwrap()
        };
        let velocity = band(&out, band_color(Source::Velocity));
        let timbre = band(&out, band_color(Source::Timbre));
        assert!((velocity.left() - (plot.left() + plot.width() * 0.25)).abs() < 0.1);
        assert!((velocity.width() - plot.width() * 0.5).abs() < 0.1);
        assert!((timbre.left() - plot.left()).abs() < 0.1);
        assert!((timbre.right() - velocity.right()).abs() < 0.1);
        for shape in overlay(&out) {
            assert!(
                base.contains_rect(shape.visual_bounding_rect()),
                "range escaped slider: {shape:?}"
            );
        }
        let overlay_index =
            out.shapes.iter().position(|s| matches!(s.shape, egui::Shape::Vec(_))).unwrap();
        let text_index = out
            .shapes
            .iter()
            .position(|s| {
                matches!(&s.shape,
            egui::Shape::Text(t) if t.galley.text() == "Opacity base")
            })
            .unwrap();
        assert!(overlay_index < text_index, "bands must stay beneath the text");
        assert!(out.shapes.iter().any(|s| matches!(&s.shape,
            egui::Shape::Text(t) if t.galley.text() == "Opacity base" && t.fallback_color == theme::text_dim())));
        for source in Source::ALL {
            assert!(out.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Path(p) if p.fill == theme::well().lerp_to_gamma(source.color(), 0.5))));
        }
        assert!(overlay(&out).iter().any(|s| matches!(s,
            egui::Shape::Rect(r) if r.fill == band_color(Source::Velocity) && r.corner_radius.nw == 0 && r.corner_radius.sw == 0 && r.corner_radius.ne > 0)));
        // Four mappings fit inside the bar, with timbre clipped at zero and
        // gain at the ceiling. Hatch strokes remain within their lane ends.
        for edge in [plot.left(), plot.right() - 5.0] {
            assert!(overlay(&out).iter().any(|s| matches!(s,
                egui::Shape::LineSegment { points, stroke } if stroke.color == theme::well()
                    && points.iter().all(|p| p.x >= edge && p.x <= edge + 5.0))));
        }
        settings.gain.opacity = Some(0.25);
        let out = frame(&ctx, &mut settings, vec![]);
        assert!(overlay(&out).iter().any(|s| matches!(s,
            egui::Shape::LineSegment { points, stroke }
                if stroke.color == band_color(Source::Gain)
                    && points[0].x >= plot.left() + plot.width() * 0.5
                    && points[1].x > points[0].x)));
        let normal_ranges = overlay(&out).to_vec();
        for pointer in [velocity.center(), texts(&out, "Velocity weight")[0].center()] {
            frame(&ctx, &mut settings, vec![egui::Event::PointerMoved(pointer)]);
            let hovered = frame(&ctx, &mut settings, vec![]);
            assert_eq!(overlay(&hovered), normal_ranges, "ranges react to hover");
        }
        frame(&ctx, &mut settings, vec![egui::Event::PointerMoved(velocity.center())]);

        // The inset ranges use the base response rather than claiming a
        // separate interaction, so dragging anywhere in them still edits base.
        frame(
            &ctx,
            &mut settings,
            vec![egui::Event::PointerButton {
                pos: velocity.center(),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let out = frame(
            &ctx,
            &mut settings,
            vec![egui::Event::PointerMoved(Pos2::new(
                base.left() + base.width() * 0.75,
                velocity.center().y,
            ))],
        );
        assert!(settings.opacity_rest > 0.6);
        let velocity = band(&out, band_color(Source::Velocity));
        assert!(
            (velocity.left() - (plot.left() + plot.width() * settings.opacity_rest)).abs() < 0.1
        );
        assert!(
            !overlay(&out).iter().any(|s| matches!(s,
            egui::Shape::LineSegment { stroke, .. } if stroke.color == theme::text())),
            "the base uses the standard slider fill, not a custom marker"
        );
    }
}
