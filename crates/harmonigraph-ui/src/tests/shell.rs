//! The root shell itself: the tab registry, and the repaint pacing
//! [`root_ui`] asks for.

use super::harness::*;
use crate::*;

#[test]
fn repainting_keeps_a_detached_extension_smooth_past_the_history_window() {
    use harmonigraph_core::{NoteEvent, SourceId};

    let mut state = fresh();
    state.picture.appearance.spectrum.show_roll = true;
    state.picture.appearance.spectrum.roll_fraction = 0.5;
    state.picture.appearance.spectrum.roll_seconds = 1.0;
    state.picture.appearance.spectrum.roll_lead_release = 2.0;
    state.picture.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
    state.picture.runtime.tracker.handle_event(NoteEvent::off(0.1, SourceId::DIRECT, 0, 60));

    assert!(crate::roll_scrolling(&state, 1.6));
    assert!(!crate::roll_scrolling(&state, 2.2));
}

/// Every tab needs an id of its own, and the title is not allowed to be its
/// source: the workspace keys each tab BODY's `Ui` on `Viewer::id`
/// (`workspace.rs`), so two tabs whose ids came from a shared title would
/// share their body state — scrolling one pane scrolls the other.
/// Variant-keyed ids are what leave a name free to be repeated across
/// sections, as a display and the settings tab holding its knobs may be.
#[test]
fn every_tab_has_its_own_id_whatever_its_title_says() {
    let mut state = fresh();
    let params = RecordingBackend::default();
    let tabs = [
        panes::Tab::Lattice,
        panes::Tab::Tuning,
        panes::Tab::LatticeSettings,
        panes::Tab::AnalyzerSettings,
        panes::Tab::Mappings,
        panes::Tab::System,
        panes::Tab::Console,
        panes::Tab::Spectral,
        panes::Tab::Spiral,
        panes::Tab::Video,
    ];
    let viewer = panes::Viewer {
        state: &mut state.picture,
        interaction: &mut state.workspace.interaction,
        params: &params,
        now: 0.0,
    };

    let ids: Vec<egui::Id> = tabs.iter().map(|&tab| viewer.id(&tab)).collect();
    for (i, a) in ids.iter().enumerate() {
        for (j, b) in ids.iter().enumerate().skip(i + 1) {
            assert_ne!(a, b, "{:?} and {:?} share a tab id", tabs[i], tabs[j]);
        }
    }
}

/// The picture panes fill their body exactly, so a scroll area around one can
/// only shift a picture that is meant to sit still.
#[test]
fn the_picture_panes_do_not_scroll() {
    let mut state = fresh();
    let params = RecordingBackend::default();
    let viewer = panes::Viewer {
        state: &mut state.picture,
        interaction: &mut state.workspace.interaction,
        params: &params,
        now: 0.0,
    };
    for tab in [panes::Tab::Lattice, panes::Tab::Spectral, panes::Tab::Spiral] {
        assert_eq!(viewer.scroll_bars(&tab), [false, false], "{tab:?} is scrollable");
    }
    // Settings panes are lists and must stay reachable in a short column, but
    // VERTICALLY only: a both-axes area gives the body unbounded width, and the
    // panes that fill the space then never report vertical overflow, so the
    // wheel can't scroll them. Horizontal off; vertical on.
    for &tab in workspace::Section::Settings.tabs() {
        assert_eq!(viewer.scroll_bars(&tab), [false, true], "{tab:?} cannot scroll vertically");
    }
}

/// A separator reads against BOTH grounds a pane can paint, because the two
/// sides of one boundary are not one surface. A picture pane fills its body with
/// the well edge to edge — `Viewer::tab_style_override` takes its margin to zero,
/// which is the arrangement the test above is the other half of — while a
/// settings pane shows the tab body's panel through. So a divider borrowed from
/// either grey vanishes along whichever boundaries happen to be drawn in it, and
/// the lattice against the analyzer, two picture panes in one grey, has no edge
/// between them at all.
///
/// A ratio and a modest one, because this palette is deep and its span is
/// narrow: the lightest grey the chrome owns is 1.93 against the well, so WCAG's
/// 3:1 for non-text is out of reach for any colour that still looks like this
/// instrument. 1.25 is the line between a divider and a coincidence — the panel
/// against the well is 1.08 and fails it, which is the case worth catching, a
/// separator dialled to a grey borrowed from the chrome around it.
#[test]
fn a_separator_reads_against_both_grounds_a_pane_can_paint() {
    /// The WCAG relative-luminance ratio between two opaque colours.
    fn contrast(a: egui::Color32, b: egui::Color32) -> f32 {
        fn luminance(c: egui::Color32) -> f32 {
            let channel = |v: u8| {
                let v = f32::from(v) / 255.0;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
        }
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    const FLOOR: f32 = 1.25;

    // Every state a separator is drawn in, not the idle one alone: a hover or a
    // drag that stopped reading would be the same bug on the frame it matters.
    for (state, color) in [
        ("idle", theme::hairline()),
        ("hovered", theme::accent_edge()),
        ("dragged", theme::accent()),
    ] {
        for (ground, fill) in [
            ("the picture panes' ground", theme::picture()),
            ("a settings pane's panel", theme::panel()),
        ] {
            let ratio = contrast(color, fill);
            assert!(
                ratio >= FLOOR,
                "a {state} separator is {ratio:.2} against {ground}, under {FLOOR} — \
                 that boundary is invisible",
            );
        }
    }
}

/// Narrow headers keep one row and put all destinations in the current-tab menu.
#[test]
fn every_settings_tab_fits_on_its_tab_bar() {
    for width in [700.0, 1000.0, 1512.0] {
        let mut state = fresh();
        let mut harness = DockHarness::at(egui::vec2(width, 700.0));
        harness.settle(&mut state);
        let output = harness.frame(&mut state, vec![]);
        let leaf = state.workspace.layout_runtime.rects[workspace::Section::Settings as usize];
        let body = state.workspace.layout_runtime.body(panes::Tab::Tuning).unwrap();
        let rail = theme::tab_bar_height(1.0);
        assert!((body.top() - leaf.top() - rail).abs() < 0.5);
        let title = output
            .shapes
            .iter()
            .find_map(|cs| match &cs.shape {
                egui::Shape::Text(t)
                    if t.galley.text() == "Tuning"
                        && leaf.contains(t.pos)
                        && t.pos.y < body.top() =>
                {
                    Some((egui::Rect::from_min_size(t.pos, t.galley.size()), cs.clip_rect))
                }
                _ => None,
            })
            .expect("current tab title");
        assert!(title.1.contains_rect(title.0));
    }
}

/// A line that stops in open space ends round: a disc half its width over each
/// end, in the stroke's own color, laid where egui actually draws the line — a
/// level line off the pixel grid is snapped onto it first, caps and all. A
/// translucent line, or one in a faded (disabled) painter, keeps egui's square
/// ends, which a disc would paint over twice.
#[test]
fn an_opaque_line_ends_round_and_a_translucent_one_square() {
    let (a, b) = (egui::pos2(20.0, 40.3), egui::pos2(80.0, 40.3));
    let caps = |color: egui::Color32, disabled: bool| {
        let stroke = egui::Stroke::new(theme::LINE_WIDTH, color);
        let mut y = a.y;
        let out = super::probe::painted_full(egui::vec2(100.0, 80.0), |ui| {
            stroke.round_center_to_pixel(ui.pixels_per_point(), &mut y);
            if disabled {
                ui.disable();
            }
            theme::round_ended(ui.painter(), vec![a, b], stroke);
        });
        assert_ne!(y, a.y, "fixture must start the line off the pixel grid");
        let line = out.shapes.iter().any(|cs| {
            matches!(cs.shape, egui::Shape::LineSegment { points, .. }
                if points == [egui::pos2(a.x, y), egui::pos2(b.x, y)])
        });
        assert!(line, "the line is not the segment egui would have drawn at {y}");
        let meshes = out
            .shapes
            .iter()
            .filter_map(|cs| match &cs.shape {
                egui::Shape::Mesh(mesh) => Some(mesh.calc_bounds()),
                _ => None,
            })
            .collect::<Vec<_>>();
        (y, meshes)
    };
    let (y, round) = caps(theme::text(), false);
    assert_eq!(round.len(), 1, "both caps are one mesh");
    let cap = egui::Vec2::splat(theme::LINE_WIDTH);
    for x in [a.x, b.x] {
        let disc = egui::Rect::from_center_size(egui::pos2(x, y), cap);
        assert!(round[0].expand(0.01).contains_rect(disc), "no cap over ({x}, {y}): {round:?}");
    }
    assert!(
        (round[0].center().y - y).abs() < 0.01,
        "the caps sit off the line egui draws: {round:?} against {y}"
    );
    assert!(
        caps(theme::text().gamma_multiply(0.5), false).1.is_empty(),
        "a translucent line got caps"
    );
    assert!(caps(theme::text(), true).1.is_empty(), "a line in a disabled section got caps");
}
