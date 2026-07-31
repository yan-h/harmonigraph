//! The settings panes as a column: that each scrolls when it overflows,
//! that no control overruns a narrow one, and the Video pane's own rows.

use crate::*;
use harmonigraph_render::wgpu::TextureFormat;
use super::harness::*;

/// Put the Notes/Console leaf back on screen, which is what the two wheel
/// harnesses below are written against: they read the settings leaf as the box
/// from the tab bar down to the 0.55 split, and the default layout opens that
/// leaf folded (see
/// [`the_default_layout_opens_with_the_two_readout_panes_folded`]) so the
/// settings column runs the whole height instead.
///
/// Unfolded rather than measured where it now is, because a taller pane is the
/// wrong pane to ask these questions of: both tests need content that
/// OVERFLOWS, and the short window they pick is short relative to this box.
fn unfold_the_readout_panes(state: &mut SharedState) {
    let path = state.dock.find_tab(&panes::Tab::Notes).expect("Notes is docked");
    state.dock[path.surface][path.node].set_collapsed(false);
}

/// Drive the REAL dock (root_ui, egui_dock, the tab body's ScrollArea and
/// all) with a wheel over `tab`'s body, and answer how far its content moved.
/// Negative = the content moved up, i.e. the pane scrolled down.
///
/// Tracks NAMED texts rather than a bounding box: egui culls whatever scrolls
/// out of the clip rect and the custom bars paint past it, so every
/// position-of-the-ink metric reports movement that isn't there (and misses
/// movement that is). The y of a string drawn in both frames cannot lie.
fn wheel_over_settings_pane(tab: panes::Tab, screen_h: f32) -> f32 {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    unfold_the_readout_panes(&mut state);
    // The settings leaf opens on Tuning; every other settings pane is a tab
    // behind it.
    let path = state.dock.find_tab(&tab).expect("{tab:?} is not in the default dock");
    state.dock.set_active_tab(path).expect("selecting the tab");
    let backend = RecordingBackend::default();
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, screen_h));
    // The top-right leaf (right of the 0.72 split, above the 0.55 one), from
    // under its tab bar down. Only shapes clipped to this are the pane's.
    let body = egui::Rect::from_min_max(
        egui::pos2(700.0, 20.0),
        egui::pos2(1000.0, screen_h * 0.55 + 2.0),
    );
    let texts = |out: &egui::FullOutput| {
        let mut map = std::collections::HashMap::new();
        for cs in &out.shapes {
            if cs.clip_rect.min.x < body.min.x
                || cs.clip_rect.min.y < body.min.y
                || cs.clip_rect.max.y > body.max.y
            {
                continue;
            }
            if let egui::Shape::Text(t) = &cs.shape {
                map.entry(t.galley.text().to_owned()).or_insert(t.pos.y);
            }
        }
        map
    };
    let mut t = 0.0;
    let mut frame = |state: &mut SharedState, events: Vec<egui::Event>| {
        t += 1.0 / 60.0;
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(t),
            events,
            ..Default::default()
        };
        texts(&ctx.run_ui(raw, |ui| root_ui(ui, state, &backend, t)))
    };
    // egui resolves the widget under the pointer from the previous pass, so
    // the pointer has to be there for a frame before the wheel arrives.
    frame(&mut state, vec![egui::Event::PointerMoved(egui::pos2(860.0, screen_h * 0.22))]);
    let before = frame(&mut state, vec![]);
    frame(
        &mut state,
        vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -3.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    // The wheel arrives smoothed over several frames.
    let mut after = before.clone();
    for _ in 0..20 {
        after = frame(&mut state, vec![]);
    }
    let mut deltas: Vec<f32> = before
        .iter()
        .filter_map(|(text, y)| after.get(text).map(|moved| moved - y))
        .collect();
    assert!(!deltas.is_empty(), "{tab:?} drew no text to measure");
    deltas.sort_by(f32::total_cmp);
    deltas[deltas.len() / 2]
}

/// Every settings pane scrolls to the wheel once its content is taller than
/// the pane. The dock hands some of them its own `ScrollArea` and others build
/// their own; from the wheel's side that must not be visible.
#[test]
fn every_settings_pane_scrolls_when_its_content_overflows() {
    // A short window, so that every one of them overflows — including Panel,
    // the shortest list of the set.
    for tab in [
        panes::Tab::Tuning,
        panes::Tab::Nodes,
        panes::Tab::Scene,
        panes::Tab::Analyzer,
        panes::Tab::Video,
        panes::Tab::Panel,
    ] {
        let moved = wheel_over_settings_pane(tab, 300.0);
        assert!(moved < -8.0, "{tab:?} did not scroll to the wheel (content moved {moved})");
    }
}

/// The window the plugin is dialled in for, and the layout it opens with,
/// between them leave the settings column with NO scroll bar of either kind:
/// not the tab bar's, when the six tab names are laid across it, and not a
/// pane's own, when its controls are stacked down it.
///
/// A scroll bar there is a scroll bar over the controls, which is the one place
/// in the window that is nothing but controls — so it reads as the settings not
/// fitting the plugin rather than as a list being long. Both halves are tight
/// enough to lose by accident: the tab bar clears its content by 76pt of the
/// 423 it gets, and the tallest pane (the Analyzer's) only stopped overflowing
/// when the Notes/Console leaf folded and handed the column the other half of
/// its height. Add a settings tab, or unfold that leaf, and one of them comes
/// back.
///
/// 1512x886 because that is the window the sizes in this UI were chosen
/// against (see `panes::lattice::REFERENCE_HEIGHT`) — this says the defaults
/// agree with each other there, not that they survive every window. Narrower
/// than about 1240 and the tab bar does overflow, which is what its own scroll
/// bar is for.
#[test]
fn the_settings_column_needs_no_scroll_bar_at_the_window_it_was_dialled_in() {
    const REFERENCE: egui::Vec2 = egui::vec2(1512.0, 886.0);
    // Left edge of the settings column: everything right of the split.
    let column_left = REFERENCE.x * crate::state::SETTINGS_SPLIT;

    for tab in [
        panes::Tab::Tuning,
        panes::Tab::Nodes,
        panes::Tab::Scene,
        panes::Tab::Analyzer,
        panes::Tab::Video,
        panes::Tab::Panel,
    ] {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        let path = state.dock.find_tab(&tab).expect("a settings tab");
        state.dock.set_active_tab(path).expect("selecting the tab");
        let backend = RecordingBackend::default();
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, REFERENCE);

        // Named texts in the column, as the wheel harnesses above track them.
        let texts = |out: &egui::FullOutput| {
            let mut map = std::collections::HashMap::new();
            for cs in &out.shapes {
                if cs.clip_rect.min.x < column_left {
                    continue;
                }
                if let egui::Shape::Text(t) = &cs.shape {
                    map.entry(t.galley.text().to_owned()).or_insert(t.pos.y);
                }
            }
            map
        };
        // egui_dock draws its tab-bar scroll bar as a 7.5pt-tall rect, and only
        // when the tabs overflow — so finding one IS the overflow.
        let scroll_bars = |out: &egui::FullOutput| {
            out.shapes
                .iter()
                .filter(|cs| match &cs.shape {
                    egui::Shape::Rect(r) => {
                        (r.rect.height() - 7.5).abs() < 0.01 && r.rect.min.x >= column_left
                    }
                    _ => false,
                })
                .count()
        };

        let mut t = 0.0;
        // Each frame answers with both readings: the named texts, and how many
        // tab-bar scroll bars the column drew.
        let mut frame = |state: &mut SharedState, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(t),
                    events,
                    ..Default::default()
                },
                |ui| root_ui(ui, state, &backend, t),
            );
            (texts(&out), scroll_bars(&out))
        };

        // The pointer has to sit over the pane for a frame before the wheel
        // lands, since egui resolves it from the previous pass.
        frame(&mut state, vec![egui::Event::PointerMoved(egui::pos2(1250.0, 300.0))]);
        let (before, bars) = frame(&mut state, vec![]);
        assert_eq!(bars, 0, "{tab:?}: the tab bar drew a scroll bar at {REFERENCE:?}");
        frame(
            &mut state,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, -3.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let mut after = before.clone();
        for _ in 0..20 {
            after = frame(&mut state, vec![]).0;
        }
        let mut deltas: Vec<f32> =
            before.iter().filter_map(|(text, y)| after.get(text).map(|m| m - y)).collect();
        assert!(!deltas.is_empty(), "{tab:?} drew no text to measure");
        deltas.sort_by(f32::total_cmp);
        let moved = deltas[deltas.len() / 2];
        assert_eq!(moved, 0.0, "{tab:?} still scrolls at {REFERENCE:?} (content moved {moved})");
    }
}

/// The Video pane drawn through the REAL dock, soloed, for a shell that can or
/// cannot record takes — the one thing that changes which section leads it.
///
/// Through `root_ui` and `DockArea` rather than calling `TabViewer::ui` on a
/// hand-built child, because the wrapping is the part under test: egui_dock
/// puts every body inside a `ScrollArea`, and that ui arrives with a
/// full-height `min_rect` where a hand-built one arrives empty. A fixture that
/// skips it cannot see the difference — see `section`.
fn video_pane_shapes(take_supported: bool) -> (Vec<egui::epaint::ClippedShape>, egui::Color32) {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.take_supported = take_supported;
    // Soloed so the Video pane's body is the only settings body on screen and
    // the first heading found is unambiguously its own.
    state.dock = egui_dock::DockState::new(vec![panes::Tab::Video]);
    let backend = RecordingBackend::default();
    let ctx = egui::Context::default();
    crate::theme::apply_theme(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 1200.0));
    let out = ctx.run_ui(
        egui::RawInput { screen_rect: Some(screen), time: Some(0.0), ..Default::default() },
        |ui| root_ui(ui, &mut state, &backend, 0.0),
    );
    // What `ui.separator()` strokes with, so the assertion names the rule
    // rather than "some line" — the tab bar draws its own, in its own colors.
    let rule = ctx.style_of(egui::Theme::Dark).visuals.widgets.noninteractive.bg_stroke.color;
    (out.shapes, rule)
}

/// Where a pane's content box ends, in the coordinates
/// [`settings_pane_at_width`] lays it out at.
fn pane_content_right(width: f32) -> f32 {
    crate::theme::PANE_INNER_MARGIN + width
}

/// The y a named text run was painted at in `shapes`, or `None`.
fn text_y(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<f32> {
    shapes.iter().find_map(|cs| match &cs.shape {
        egui::Shape::Text(t) if t.galley.text() == needle => Some(t.pos.y),
        _ => None,
    })
}

/// The Video pane's first heading has no rule above it — there is nothing
/// above it to be separated from, and a rule there reads as the pane hanging
/// off a line.
///
/// The Video pane and not every pane, because it is the only one that leads
/// with a `section` call: the rest draw content first, so their rules all
/// separate something. What this pins is the MECHANISM in `section`, which
/// any pane gets — so a new pane that leads with a section needs no test of
/// its own, and this one has to keep working for it to stay that way.
///
/// Both shells, because which section leads the Video pane depends on the
/// shell: a host can record takes, so Record leads; the standalone cannot, so
/// `render_settings` returns early and Frame leads instead. `section` decides
/// it from what has been drawn rather than from the caller, and this is what
/// holds it to that for the case the caller could not have known.
#[test]
fn the_video_pane_does_not_start_with_a_rule() {
    // Which section leads, per shell: a host can record takes, so Record
    // leads; the standalone cannot, so `render_settings` returns early.
    for (take_supported, leads) in [(true, "Record"), (false, "Frame")] {
        let (shapes, rule) = video_pane_shapes(take_supported);
        let heading = text_y(&shapes, leads)
            .unwrap_or_else(|| panic!("the Video pane drew no {leads:?} heading"));
        // Only rules BELOW the tab bar are the pane's own. The dock's chrome
        // draws lines of its own above the body, in its own colors, and those
        // are not this test's business — hence matching on the separator
        // stroke rather than on any line segment.
        let above = shapes.iter().any(|cs| match &cs.shape {
            egui::Shape::LineSegment { points, stroke } => {
                stroke.color == rule && points[0].y < heading
            }
            _ => false,
        });
        assert!(
            !above,
            "take_supported={take_supported}: a rule sits above {leads:?}, the pane's \
             first heading, at y {heading}"
        );
    }
}

fn bar_track_widths(shapes: &[egui::epaint::ClippedShape]) -> Vec<f32> {
    let well = crate::theme::well();
    shapes
        .iter()
        .filter_map(|cs| match &cs.shape {
            egui::Shape::Rect(r) if r.fill == well && (r.rect.height() - 20.0).abs() < 0.6 => {
                Some(r.rect.width())
            }
            _ => None,
        })
        .collect()
}

/// Every bar in a settings pane is the same length, and that length is the
/// column's — so dragging the column narrower narrows all of them together.
///
/// What breaks it is invisible in the code that draws a bar, which is why this
/// is pinned rather than left to reading: egui's `Region::expand_to_include_rect`
/// unions `max_rect` as well as `min_rect`, so any control that overruns the
/// column widens the column for everything BELOW it, and a bar sizing itself
/// from bare `available_width` inherits the overrun as a floor it cannot shrink
/// past. Each bar's minimum length is then the width of the widest thing above
/// it — five different minimums down one pane, the bars under a wide row running
/// their value readout off the pane edge while the bars above it compress
/// properly. `widgets::bar_width` is the answer, and the reason it measures the
/// clip rect rather than trusting the layout.
///
/// Swept past the width where the pane's other controls stop fitting on purpose.
/// Above about 100pt nothing overruns at all (see below), so those widths would
/// pass whether a bar clamped itself or not; 100 and 80, where the record button
/// and the Options field have nowhere left to go, are where the clamp is the
/// only thing holding the bars level.
#[test]
fn every_bar_in_a_settings_pane_is_the_width_of_the_pane() {
    for width in [400.0f32, 240.0, 160.0, 120.0, 100.0, 80.0] {
        for tab in SETTINGS_TABS {
            for &projection in projections_for(tab) {
                let widths = bar_track_widths(&settings_pane_at_width(tab, width, projection));
                for bar in &widths {
                    assert!(
                        (bar - width).abs() < 1.0,
                        "{tab:?}/{projection:?} at {width}pt drew a {bar}pt bar \
                         (all of {widths:?})"
                    );
                }
            }
        }
    }
    // The sniffing above finds nothing if the bars stop being painted this way,
    // and a test that measures nothing passes. The Tuning pane is the deepest
    // stack of bars in the dock.
    let bars =
        bar_track_widths(&settings_pane_at_width(panes::Tab::Tuning, 400.0, PROJECTIONS[0])).len();
    assert!(bars >= 10, "only found {bars} bar tracks in the Tuning pane; has the paint changed?");
}

/// The render bar fills to the share of frames done — which is the whole
/// reason it is a bar and not another sentence in the status line, since a
/// render is minutes long and the sentence never changes while it runs.
///
/// The fraction is also what tells it apart from the `ValueBar` beside it,
/// which paints the same accent fill. The split bar fills to its position in
/// its RANGE, not to its value: 0.20 across 0.05..=0.95 is a sixth of the
/// track, against the fixture render's eighth. They have to stay further
/// apart than the tolerance below, so moving the split's default or its range
/// close to an eighth is what would make this pass on the wrong bar.
#[test]
fn the_render_bar_fills_to_the_share_of_frames_done() {
    const WIDTH: f32 = 400.0;
    let shapes = settings_pane_at_width(panes::Tab::Video, WIDTH, PROJECTIONS[0]);
    let share = FIXTURE_RENDER.fraction().expect("the fixture render knows its total");
    let fills: Vec<f32> = shapes
        .iter()
        .filter_map(|cs| match &cs.shape {
            egui::Shape::Rect(r)
                if r.fill == crate::theme::accent_fill()
                    && (r.rect.height() - 20.0).abs() < 0.6 =>
            {
                Some(r.rect.width() / WIDTH)
            }
            _ => None,
        })
        .collect();
    assert!(
        fills.iter().any(|filled| (filled - share).abs() < 0.01),
        "no bar filled to {share} of the column; found {fills:?}"
    );
}

/// Before the renderer has said how many frames it is composing there is no
/// share to draw, and an empty track says "starting" where a track filled to
/// zero would say "none of it done yet" — a claim nothing has made.
#[test]
fn a_render_that_has_not_announced_its_total_has_no_fraction() {
    assert_eq!(RenderProgress { done: 0, total: 0 }.fraction(), None);
    assert_eq!(RenderProgress { done: 90, total: 0 }.fraction(), None);
    assert_eq!(RenderProgress { done: 1, total: 4 }.fraction(), Some(0.25));
    // A renderer that overshot its own estimate still fills a bar, not past it.
    assert_eq!(RenderProgress { done: 9, total: 4 }.fraction(), Some(1.0));
}

/// No settings pane's controls run out past the column, at any width worth
/// dragging one to. Off the pane edge a control cannot be read, clicked, or
/// dragged to its end, and horizontal scrolling is deliberately off in the dock
/// (see `panes::Viewer::scroll_bars`), so there is no way to reach it.
///
/// Three things hold it: rows wrap, and so do the labels of the buttons in them
/// (`widgets::button_row`); bars take the column's visible width
/// (`widgets::bar_width`); and a bar's name elides against its own value readout
/// instead of running over it and out of the pane.
///
/// 120pt is the narrowest pinned because it is the last width where everything
/// still fits. Below about 100 what is left is widgets that wrap nothing and
/// have nowhere to wrap to — the record button, a `toggle_switch` label, the
/// Options field — and the answer there would be to elide those too, which costs
/// every reader something to buy back a column nobody drags to.
///
/// The column opens at around 423pt (`state::SETTINGS_SPLIT` of the reference
/// window) and fits there, which is why this went unnoticed: the overrun starts
/// somewhere under 400, and by 300 the Tuning pane was running 32pt of bar off
/// its own edge. It is a resize bug, so the sweep is the test.
#[test]
fn no_settings_pane_overruns_a_narrow_column() {
    for width in [400.0f32, 300.0, 240.0, 200.0, 160.0, 120.0] {
        let edge = pane_content_right(width);
        // The pane's own clip is the tab body, a margin wider than the content
        // box on each side.
        let body_right = edge + crate::theme::PANE_INNER_MARGIN;
        let panes = SETTINGS_TABS
            .into_iter()
            .flat_map(|tab| projections_for(tab).iter().map(move |&p| (tab, p)));
        for (tab, projection) in panes {
            let shapes = settings_pane_at_width(tab, width, projection);
            let over_edge = |cs: &egui::epaint::ClippedShape| {
                let rect = cs.shape.visual_bounding_rect();
                // Shapes that carry no geometry answer with an inverted or
                // infinite rect; egui's own `is_finite` lets those through.
                if !rect.is_finite() || rect.width() > 1.0e4 {
                    return None;
                }
                // A widget that set its own clip, tighter than the body, is
                // managing its own overflow — a single-line text box scrolls
                // its content inside the field, so its galley is routinely
                // wider than the box and correctly cut off there. Only the
                // body's own clip means "cut off by the pane", which is the
                // thing that can be neither reached nor read.
                if cs.clip_rect.right() < body_right - 0.5 {
                    return None;
                }
                (rect.right() - edge > 1.0).then(|| rect.right() - edge)
            };
            let worst = shapes
                .iter()
                .filter_map(|cs| over_edge(cs).map(|over| (over, cs)))
                .max_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(over, cs)| {
                    let what = match &cs.shape {
                        egui::Shape::Text(t) => format!("{:?}", t.galley.text()),
                        other => format!("{other:?}").chars().take(40).collect(),
                    };
                    (over, what)
                });
            assert!(
                worst.is_none(),
                "{tab:?}/{projection:?} at {width}pt ran {:?} past the pane edge",
                worst.unwrap()
            );
        }
    }
}

/// A drag whose release never arrives must not take the wheel down with it.
///
/// egui gates every `ScrollArea` on `dragged_id().is_none()` — globally, not
/// per area — so one stale drag stops the wheel in EVERY settings pane at once.
/// In a plugin window that is a routine event rather than an exotic one: let go
/// outside the editor, or let the host take focus mid-drag, and the release is
/// delivered somewhere that is not us. Both gestures below are ones a person
/// actually makes: panning the Analyzer's pitch range out past its edge, and
/// dragging any settings bar.
#[test]
fn a_drag_that_loses_its_release_does_not_strand_the_wheel() {
    // Default dock: the Analyzer picture is the column at x ~518..720, the
    // settings leaf is top-right.
    for (what, at) in [("the analyzer picture", 600.0f32), ("a settings bar", 860.0)] {
        for lose_it in [Lose::Pointer, Lose::Focus] {
            let moved = scroll_settings_after_lost_drag(egui::pos2(at, 200.0), lose_it);
            assert!(
                moved < -8.0,
                "a drag on {what} that lost its release to {lose_it:?} left the settings \
                 pane unscrollable (content moved {moved})",
            );
        }
    }
}

/// How the release goes missing: the pointer leaves the editor, or the host
/// takes focus while the button is down.
#[derive(Clone, Copy, Debug)]
enum Lose {
    Pointer,
    Focus,
}

/// Press and drag at `from`, lose the release, then wheel over the settings
/// pane and answer how far its content moved.
fn scroll_settings_after_lost_drag(from: egui::Pos2, lose: Lose) -> f32 {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    unfold_the_readout_panes(&mut state);
    let path = state.dock.find_tab(&panes::Tab::Analyzer).expect("the Analyzer settings tab");
    state.dock.set_active_tab(path).expect("selecting the tab");
    let backend = RecordingBackend::default();
    let ctx = egui::Context::default();
    let screen_h = 500.0;
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, screen_h));
    let body =
        egui::Rect::from_min_max(egui::pos2(700.0, 20.0), egui::pos2(1000.0, screen_h * 0.55 + 2.0));
    // Named texts inside the settings body, as `wheel_over_settings_pane` does:
    // the y of a string drawn in both frames is the one metric a clip rect and
    // a culled shape cannot lie about.
    let texts = |out: &egui::FullOutput| {
        let mut map = std::collections::HashMap::new();
        for cs in &out.shapes {
            if cs.clip_rect.min.x < body.min.x
                || cs.clip_rect.min.y < body.min.y
                || cs.clip_rect.max.y > body.max.y
            {
                continue;
            }
            if let egui::Shape::Text(t) = &cs.shape {
                map.entry(t.galley.text().to_owned()).or_insert(t.pos.y);
            }
        }
        map
    };
    let mut t = 0.0;
    let mut frame = |state: &mut SharedState, events: Vec<egui::Event>| {
        t += 1.0 / 60.0;
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(t),
            events,
            ..Default::default()
        };
        texts(&ctx.run_ui(raw, |ui| root_ui(ui, state, &backend, t)))
    };
    let press = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    // Hover, press, drag — and then the release goes missing.
    frame(&mut state, vec![egui::Event::PointerMoved(from)]);
    frame(&mut state, vec![press(from, true)]);
    frame(&mut state, vec![egui::Event::PointerMoved(from + egui::vec2(0.0, 40.0))]);
    frame(
        &mut state,
        match lose {
            Lose::Pointer => vec![egui::Event::PointerGone],
            Lose::Focus => vec![egui::Event::WindowFocused(false)],
        },
    );

    // Back over the settings pane, wheel, and see whether anything moves.
    let settings = egui::pos2(860.0, 130.0);
    frame(&mut state, vec![egui::Event::PointerMoved(settings)]);
    let before = frame(&mut state, vec![]);
    frame(
        &mut state,
        vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -3.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    // The wheel arrives smoothed over several frames.
    let mut after = before.clone();
    for _ in 0..20 {
        after = frame(&mut state, vec![]);
    }
    let mut deltas: Vec<f32> =
        before.iter().filter_map(|(text, y)| after.get(text).map(|m| m - y)).collect();
    assert!(!deltas.is_empty(), "the settings pane drew no text to measure");
    deltas.sort_by(f32::total_cmp);
    deltas[deltas.len() / 2]
}

/// The Video pane scrolls at a workable size, rather than swallowing the slack
/// with its preview.
///
/// It was the one settings pane the wheel did nothing in, at every size a
/// person would actually use. The preview took `available_size()`, so the
/// pane's content measured *exactly* the pane however short the pane got —
/// the dock's `ScrollArea` never saw anything sticking out to scroll, and the
/// preview shrank towards a sliver instead of the controls staying reachable.
#[test]
fn the_video_pane_scrolls_instead_of_squeezing_its_preview() {
    let moved = wheel_over_settings_pane(panes::Tab::Video, 600.0);
    assert!(moved < -8.0, "the Video pane did not scroll to the wheel (content moved {moved})");
}
