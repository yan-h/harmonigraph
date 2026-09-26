//! Target-owned controls and possible-reach previews for note mappings.
//! No live-note selection or cached derived state: ranges use the same sum
//! as the picture, and are painted after this frame's controls have edited it.

use egui::{Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2};
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
        // Stable identities, made with the skin's own accent color model.
        let hue = match self {
            Self::Velocity => 240.0,
            Self::Pressure => 85.0,
            Self::Timbre => 310.0,
            Self::Gain => 145.0,
        };
        let [r, g, b] = harmonigraph_scene::skin::accent_color(hue, 0.65);
        Color32::from_rgb(r, g, b)
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
                source.setting(&mut single).target = IntensityTarget::Off;
            }
        }
        single.reach(target)
    }
}

fn target_name(target: IntensityTarget) -> &'static str {
    match target {
        IntensityTarget::Off => "Off",
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
        IntensityTarget::Off => unreachable!(),
    }
}

pub(super) fn show(ui: &mut Ui, settings: &mut IntensitySettings) {
    widgets::weak(ui, "Bands show possible reach. Stripes mark clipping; dashes mark gain boosts.");
    // Defer moves until every group is drawn, so a source cannot appear twice
    // or disappear mid-frame depending on which group its menu belongs to.
    let mut route = None;
    for target in [IntensityTarget::Opacity, IntensityTarget::Thickness, IntensityTarget::Glow] {
        ui.push_id(target_name(target), |ui| {
            group(ui, settings, target, &mut route);
        });
    }
    if let Some((source, target)) = route {
        source.setting(settings).target = target;
        ui.ctx().request_repaint();
    }
}

fn group(
    ui: &mut Ui,
    settings: &mut IntensitySettings,
    target: IntensityTarget,
    route: &mut Option<(Source, IntensityTarget)>,
) {
    let scale = theme::ui_scale(ui.ctx());
    ui.horizontal_wrapped(|ui| {
        widgets::label(ui, target_name(target));
        egui::containers::menu::MenuButton::new("Add mapping")
            .config(egui::containers::menu::MenuConfig::new().style(widgets::menu_style(ui.ctx())))
            .ui(ui, |ui| {
                for source in Source::ALL {
                    let current = source.setting(settings).target;
                    let label = if current != IntensityTarget::Off && current != target {
                        format!("{} — move from {}", source.name(), target_name(current))
                    } else {
                        source.name().to_owned()
                    };
                    if ui.add_enabled(current != target, egui::Button::new(label)).clicked() {
                        *route = Some((source, target));
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
        .filter(|source| source.setting(settings).target == target)
        .collect();
    // Reserve only the height. The base bar's actual rect supplies the common
    // horizontal scale (including the settings column's own width clamp).
    let preview = (!sources.is_empty()).then(|| {
        ui.allocate_exact_size(
            Vec2::new(0.0, (sources.len() as f32 + 1.0) * 6.0 * scale),
            Sense::hover(),
        )
        .0
    });
    let base = match target {
        IntensityTarget::Opacity => ValueBar::new(&mut settings.opacity_rest, 0.0..=1.0, "Opacity base")
            .show(ui).on_hover_text("Starting opacity, even with no mappings. Only timbre can reduce it. A base of 1 leaves no room for positive additions."),
        IntensityTarget::Glow => ValueBar::new(&mut settings.glow_base, 0.0..=BLOOM_MAX, "Bloom base")
            .unit(1.0, "×").show(ui).on_hover_text("Starting note bloom, even with no mappings. Mappings can add bloom from base 0. The separate lattice background glow is unchanged."),
        IntensityTarget::Thickness => ValueBar::new(&mut settings.thickness_base, 0.0..=settings.thickness_max, "Thickness base")
            .unit(1.0, "×").show(ui).on_hover_text("Starting thickness. 1× is Ribbon width in the Analyzer and the MIDI layer width in the Lattice. The mappings add multiples of those same reference widths; a hidden layer remains hidden."),
        IntensityTarget::Off => unreachable!(),
    };
    let mut highlighted = None;
    for &source in &sources {
        let response = ui
            .push_id(source.name(), |ui| {
                ui.horizontal(|ui| {
                    let (swatch, _) = ui.allocate_exact_size(
                        Vec2::new(8.0 * scale, theme::row_height(scale)),
                        Sense::hover(),
                    );
                    ui.painter().rect_filled(
                        Rect::from_center_size(swatch.center(), Vec2::new(8.0, 4.0) * scale),
                        0.0,
                        source.color(),
                    );
                    let width = (ui.available_width() - 24.0 * scale - ui.spacing().item_spacing.x)
                        .max(0.0);
                    let label = format!("{} weight", source.name());
                    let response = ui
                        .allocate_ui_with_layout(
                            Vec2::new(width, theme::row_height(scale)),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                let mut bar = ValueBar::new(
                                    &mut source.setting(settings).weight,
                                    0.0..=INTENSITY_WEIGHT_MAX,
                                    &label,
                                );
                                if source == Source::Timbre {
                                    bar = bar.display(|v| format!("±{v:.2}"));
                                }
                                bar.show(ui).on_hover_text(source.hint())
                            },
                        )
                        .inner;
                    if ui
                        .add_sized(
                            Vec2::new(24.0 * scale, theme::row_height(scale)),
                            egui::Button::new("×"),
                        )
                        .on_hover_text(format!(
                            "Remove {} from {}",
                            source.name(),
                            target_name(target)
                        ))
                        .clicked()
                    {
                        *route = Some((source, IntensityTarget::Off));
                    }
                    response
                })
                .inner
            })
            .inner;
        if response.hovered() || response.dragged() || response.has_focus() {
            highlighted = Some(source);
        }
    }
    if let Some(preview) = preview {
        let reach = settings.reach(target);
        let (value, ceiling) = limits(settings, target);
        let rect = Rect::from_min_max(
            Pos2::new(base.rect.left(), preview.top()),
            Pos2::new(base.rect.right(), preview.bottom()),
        );
        let hover = ui.interact(rect, ui.id().with("reach"), Sense::hover());
        if let Some(pointer) = hover.hover_pos() {
            highlighted = sources.get(((pointer.y - rect.top()) / (6.0 * scale)) as usize).copied();
        }
        paint_reach(
            ui.painter(),
            rect,
            base.rect.top(),
            settings,
            target,
            &sources,
            highlighted,
            scale,
        );
        let suffix = if target == IntensityTarget::Opacity { "" } else { "×" };
        let boosts = if reach.gain_boosts { " at unity; gain boosts →" } else { "" };
        widgets::weak(
            ui,
            format!(
                "Together {:.2}–{:.2}{suffix}{boosts}",
                reach.min.clamp(0.0, ceiling),
                reach.max.clamp(0.0, ceiling)
            ),
        );
        hover.on_hover_text(format!("Possible reach from base {value:.2}. Colored bands show each source alone; the neutral band shows their sum. Striped ends are clipped. Dashed extensions show gain above unity. These are configured ranges, not live notes."));
    } else {
        widgets::weak(ui, "No mappings");
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_reach(
    painter: &egui::Painter,
    rect: Rect,
    base_top: f32,
    settings: &IntensitySettings,
    target: IntensityTarget,
    sources: &[Source],
    highlighted: Option<Source>,
    scale: f32,
) {
    if !painter.is_visible() || !painter.clip_rect().intersects(rect) {
        return;
    }
    let (base, ceiling) = limits(settings, target);
    for (i, &source) in sources.iter().enumerate() {
        let color = if highlighted.is_some_and(|s| s != source) {
            source.color().gamma_multiply(0.3)
        } else {
            source.color()
        };
        let lane = Rect::from_min_size(
            rect.min + Vec2::new(0.0, i as f32 * 6.0 * scale),
            Vec2::new(rect.width(), 4.0 * scale),
        );
        paint_band(painter, lane, source.reach(settings, target), ceiling, color, scale);
    }
    let total = Rect::from_min_size(
        rect.min + Vec2::new(0.0, sources.len() as f32 * 6.0 * scale),
        Vec2::new(rect.width(), 3.0 * scale),
    );
    paint_band(painter, total, settings.reach(target), ceiling, theme::text_dim(), scale);
    let x = rect.left() + rect.width() * (base / ceiling).clamp(0.0, 1.0);
    painter.line_segment(
        [Pos2::new(x, rect.top()), Pos2::new(x, base_top)],
        Stroke::new(scale, theme::text()),
    );
}

fn paint_band(
    painter: &egui::Painter,
    rect: Rect,
    reach: IntensityReach,
    ceiling: f32,
    color: Color32,
    scale: f32,
) {
    let x = |value: f32| rect.left() + rect.width() * (value / ceiling).clamp(0.0, 1.0);
    painter.rect_filled(rect, 0.0, theme::well());
    let span = Rect::from_min_max(
        Pos2::new(x(reach.min), rect.top()),
        Pos2::new(x(reach.max), rect.bottom()),
    );
    if span.width() > 0.0 {
        painter.rect_filled(span, 0.0, color);
    }
    if reach.gain_boosts {
        let mut start = x(reach.max);
        while start < rect.right() {
            painter.line_segment(
                [
                    Pos2::new(start, rect.center().y),
                    Pos2::new((start + 3.0 * scale).min(rect.right()), rect.center().y),
                ],
                Stroke::new(scale, color),
            );
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
            painter.rect_filled(end, 0.0, color);
            let clip = painter.with_clip_rect(painter.clip_rect().intersect(end));
            for i in 0..4 {
                let x = at + i as f32 * 3.0 * scale;
                clip.line_segment(
                    [Pos2::new(x, rect.bottom()), Pos2::new(x + rect.height(), rect.top())],
                    Stroke::new(scale, theme::well()),
                );
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
    fn target_menus_move_and_remove_a_source_without_changing_its_weight() {
        let ctx = crate::tests::probe::themed();
        let mut settings = IntensitySettings {
            velocity: IntensitySource { target: IntensityTarget::Opacity, weight: 0.4 },
            ..Default::default()
        };
        let out = frame(&ctx, &mut settings, vec![]);
        let add = texts(&out, "Add mapping");
        assert_eq!(add.len(), 3);
        click(&ctx, &mut settings, add[1].center());
        let menu = frame(&ctx, &mut settings, vec![]);
        let item = texts(&menu, "Velocity — move from Opacity")[0];
        click(&ctx, &mut settings, item.center());
        assert_eq!(settings.velocity.target, IntensityTarget::Thickness);
        assert_eq!(settings.velocity.weight, 0.4);
        let out = frame(&ctx, &mut settings, vec![]);
        assert_eq!(texts(&out, "Velocity weight").len(), 1);
        let remove = texts(&out, "×");
        assert_eq!(remove.len(), 1);
        click(&ctx, &mut settings, remove[0].center());
        assert_eq!(settings.velocity.target, IntensityTarget::Off);
        assert_eq!(settings.velocity.weight, 0.4);
        let out = frame(&ctx, &mut settings, vec![]);
        assert!(texts(&out, "Velocity weight").is_empty());
    }

    #[test]
    fn bands_share_the_base_scale_and_highlight_the_hovered_source() {
        let ctx = crate::tests::probe::themed();
        let mut settings = IntensitySettings {
            opacity_rest: 0.25,
            velocity: IntensitySource { target: IntensityTarget::Opacity, weight: 0.5 },
            pressure: IntensitySource { target: IntensityTarget::Opacity, weight: 0.25 },
            timbre: IntensitySource { target: IntensityTarget::Opacity, weight: 0.5 },
            gain: IntensitySource { target: IntensityTarget::Opacity, weight: 0.25 },
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
        let band = |out: &egui::FullOutput, color: Color32| {
            out.shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(r)
                        if r.fill == color
                            && r.rect.width() > 10.0
                            && (r.rect.height() - 4.0).abs() < 0.01 =>
                    {
                        Some(r.rect)
                    }
                    _ => None,
                })
                .unwrap()
        };
        let velocity = band(&out, Source::Velocity.color());
        let timbre = band(&out, Source::Timbre.color());
        assert!((velocity.left() - (base.left() + base.width() * 0.25)).abs() < 0.1);
        assert!((velocity.width() - base.width() * 0.5).abs() < 0.1);
        assert!((timbre.left() - base.left()).abs() < 0.1);
        assert!((timbre.right() - velocity.right()).abs() < 0.1);
        assert!(!texts(&out, "Together 0.00–1.00 at unity; gain boosts →").is_empty());
        assert!(out.shapes.iter().any(|s| matches!(&s.shape,
            egui::Shape::LineSegment { points, stroke }
                if stroke.color == Source::Gain.color()
                    && points[0].x >= base.left() + base.width() * 0.5
                    && points[1].x > points[0].x)));
        // This fixture clips timbre at zero and the total at both ends; the
        // hatch strokes must actually be drawn through narrow edge clips.
        for x in [base.left(), base.right() - 5.0] {
            assert!(out.shapes.iter().any(|s| (s.clip_rect.left() - x).abs() < 0.1
                && (s.clip_rect.width() - 5.0).abs() < 0.1
                && matches!(s.shape, egui::Shape::LineSegment { .. })));
        }
        frame(&ctx, &mut settings, vec![egui::Event::PointerMoved(velocity.center())]);
        let out = frame(&ctx, &mut settings, vec![]);
        assert_eq!(band(&out, Source::Velocity.color()), velocity);
        assert_eq!(band(&out, Source::Timbre.color().gamma_multiply(0.3)), timbre);
    }
}
