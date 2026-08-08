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

/// The Nodes pane's Shape bar draws the curve the NOTES run on, not a second
/// copy of the formula that happens to look like it.
///
/// The whole value of a preview is that it cannot disagree with what it
/// previews, and a disagreement here is invisible: a line that bends the wrong
/// amount still looks like a curve, and the number beside it reads 0.35 either
/// way. So the bar is painted for real and every point on the line is checked
/// against `Envelope` — the one place the shape is written.
///
/// The line's own ends calibrate the plot box, rather than the paint constants
/// being restated here: an approach starts at nothing and lands on full, so the
/// first point IS the floor and the last IS the ceiling. That leaves the test
/// measuring the SHAPE of the line and nothing about where the widget chose to
/// put it — inset, height and scale are all free to change under it.
///
/// The envelope comes through [`ViewConfig::envelope`], which is where a NOTE's
/// curve comes from, and that is the half that makes the name true. Read off
/// the `fade_shape` field instead and the test compares the bar against the
/// number it was handed rather than against the notes: put a mapping between
/// the two — a rescale of how hard the setting bends, say — and the picture
/// drifts from the lattice with this still green.
#[test]
fn the_shape_bars_preview_is_the_curve_the_notes_run_on() {
    let shapes: Vec<egui::Shape> = settings_pane_at_width(
        panes::Tab::Nodes,
        320.0,
        harmonigraph_scene::Projection::default(),
    )
    .into_iter()
    .map(|cs| cs.shape)
    .collect();
    let points = crate::widgets::curve_points(&shapes);
    assert!(points.len() > 8, "the Nodes pane drew {} preview points", points.len());

    // A unit-length arrival, which is the whole curve: the shape lives in the
    // fraction and not in the seconds, so any positive duration draws it.
    let envelope = harmonigraph_scene::ViewConfig::default()
        .envelope(&harmonigraph_scene::FrameParams { fade_time: 1.0, ..Default::default() });
    let (left, right) = (points[0].x, points[points.len() - 1].x);
    let (floor, ceiling) = (points[0].y, points[points.len() - 1].y);
    assert!(right > left, "the line runs backwards, {left} to {right}");
    assert!(floor > ceiling, "the line runs downward: it is an arrival, and rises");
    // A fifth of a point, which sounds arbitrary and is not: the widget and the
    // line below compute the same expression, so the residual is f32 rounding
    // and nothing else, and the tolerance is only there to name that. What it
    // must NOT be is a fraction of the picture — the plot is 13 points tall, so
    // the half-point that reads as "close enough on screen" is 0.04 in level,
    // and a preview quietly softened to `shape * 0.9` sits inside it.
    for point in &points {
        let p = (point.x - left) / (right - left);
        let want = floor - (floor - ceiling) * envelope.attack(p as f64, 0.0);
        assert!(
            (point.y - want).abs() < 0.02,
            "at {p} through the transition the line is at {} and the envelope at {want}",
            point.y,
        );
    }
    // A straight line satisfies the loop above at shape 0 and nowhere else, so
    // the fresh view being curved is what gives it teeth.
    assert!(
        envelope.shape > 0.0,
        "a fresh view fades on a straight line; the test above proves nothing",
    );
}

/// The Video pane drawn through the REAL dock, soloed, for a shell that can or
/// cannot record takes — the one thing that changes which section leads it.
///
/// Through `root_ui` and `DockArea` rather than calling `TabViewer::ui` on a
/// hand-built child, because the wrapping is the part under test: egui_dock
/// puts every body inside a `ScrollArea`, and that ui arrives with a
/// full-height `min_rect` where a hand-built one arrives empty. A fixture that
/// skips it cannot see the difference — see `section`.
fn video_pane_shapes(supported: bool) -> (Vec<egui::epaint::ClippedShape>, egui::Color32) {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.take.supported = supported;
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
    for (supported, leads) in [(true, "Record"), (false, "Frame")] {
        let (shapes, rule) = video_pane_shapes(supported);
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
            "take.supported={supported}: a rule sits above {leads:?}, the pane's \
             first heading, at y {heading}"
        );
    }
}

/// The bar tracks a pane drew, by width.
///
/// A `ValueBar`/`RangeBar` track is a `theme::ROW_HEIGHT`-tall rect in `well()`,
/// which the accent fill over it does not answer to — that is the same height in
/// a different color. The record button's panel does: it is a control in a
/// settings row, so it is a row high like everything else in one, and it is
/// painted in the same track color. The dot inside it is what tells the two
/// apart, and it is read out of the paint rather than the panel being skipped
/// for having an odd width — a width is exactly what is under test here, so
/// excusing a rect for being an odd length would excuse the bug.
fn bar_track_widths(shapes: &[egui::epaint::ClippedShape]) -> Vec<f32> {
    let well = crate::theme::well();
    let dots: Vec<egui::Pos2> = shapes
        .iter()
        .filter_map(|cs| match &cs.shape {
            egui::Shape::Circle(c) => Some(c.center),
            _ => None,
        })
        .collect();
    shapes
        .iter()
        .filter_map(|cs| match &cs.shape {
            egui::Shape::Rect(r)
                if r.fill == well
                    && (r.rect.height() - crate::theme::ROW_HEIGHT).abs() < 0.6
                    && !dots.iter().any(|&dot| r.rect.contains(dot)) =>
            {
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
                // One bar per gradient is deliberately shorter: the spectrum
                // track, which gives the left end of its row to the flip
                // button. It still narrows with the column, which is what this
                // is about, so it is allowed its own length rather than excused
                // from the sweep.
                //
                // COUNTED, not merely permitted. A sweep that accepts either
                // length from any bar accepts a spectrum track that never
                // reserved the button's width at all — it comes out at the
                // column's own length and passes on the first alternative,
                // with the button painted over its left end. So the short
                // length is allowed exactly once per pane that holds one, and
                // two panes now do: Nodes dials the lattice's pitch gradient
                // and Analyzer the heatmap's level gradient, on the same three
                // bars over the same type.
                let track = crate::widgets::spectrum_track_width(width, 1.0);
                let mut short = 0;
                for bar in &widths {
                    if (bar - width).abs() < 1.0 {
                        continue;
                    }
                    short += 1;
                    assert!(
                        (bar - track).abs() < 1.0,
                        "{tab:?}/{projection:?} at {width}pt drew a {bar}pt bar, \
                         neither the column nor the spectrum track's {track}pt \
                         (all of {widths:?})"
                    );
                }
                let want =
                    usize::from(matches!(tab, panes::Tab::Nodes | panes::Tab::Analyzer));
                assert_eq!(
                    short, want,
                    "{tab:?}/{projection:?} at {width}pt drew {short} short bars, not {want} \
                     (all of {widths:?})"
                );
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

/// A bar dragged off the window keeps the bar, and the release outside gives
/// it back.
///
/// The other side of [`a_drag_that_loses_its_release_does_not_strand_the_wheel`]:
/// what ends a drag there is the shell SAYING the pointer is gone, and a
/// pointer merely standing outside the window says nothing of the kind. The
/// plugin shell holds that claim back for as long as a button is down
/// (`mouse_exited` in the vendored baseview), because AppKit goes on delivering
/// the drag to the view the press landed in — so the value must go on
/// following the pointer past the edge, and pin at the end of its range rather
/// than letting go of a bar the hand is still holding.
///
/// A rule written against the POSITION instead — "outside the screen rect, so
/// stop dragging" — passes every stranded-wheel test and fails this one, which
/// is the only reason it is here.
#[test]
fn a_bar_dragged_past_the_window_edge_keeps_tracking_the_pointer() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    unfold_the_readout_panes(&mut state);
    let path = state.dock.find_tab(&panes::Tab::Analyzer).expect("the Analyzer settings tab");
    state.dock.set_active_tab(path).expect("selecting the tab");
    let backend = RecordingBackend::default();
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 500.0));
    // The settings leaf, whose bars run the width of the column at x ~700..1000.
    let body = egui::Rect::from_min_max(egui::pos2(700.0, 20.0), egui::pos2(1000.0, 277.0));
    // Where a named bar was drawn, so the gesture takes hold of a bar this test
    // can name rather than of whatever a fixed coordinate lands on. A bar draws
    // its name inside its own rectangle, at the left end.
    let bar_named = |out: &egui::FullOutput, name: &str| {
        out.shapes.iter().find_map(|cs| match &cs.shape {
            egui::Shape::Text(t)
                if body.contains(cs.clip_rect.min) && t.galley.text().starts_with(name) =>
            {
                Some(t.pos)
            }
            _ => None,
        })
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
        ctx.run_ui(raw, |ui| root_ui(ui, state, &backend, t))
    };
    let press = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    // Smoothing: a plain 0..=0.9 bar, so where the pointer is says what the
    // value should be, and the far end of the range is what an off-window drag
    // to the right must arrive at.
    let out = frame(&mut state, vec![]);
    let name =
        bar_named(&out, "Smoothing").expect("the Smoothing bar is drawn in the Analyzer tab");
    let on_the_bar = name + egui::vec2(2.0, 4.0);
    let before = state.spectrum_config.smoothing;
    frame(&mut state, vec![egui::Event::PointerMoved(on_the_bar)]);
    frame(&mut state, vec![press(on_the_bar, true)]);
    frame(&mut state, vec![egui::Event::PointerMoved(on_the_bar + egui::vec2(60.0, 0.0))]);
    assert!(ctx.dragged_id().is_some(), "the press on the Smoothing bar started no drag");
    let inside = state.spectrum_config.smoothing;
    assert!(inside != before, "the bar did not follow the pointer inside the window");

    // Out past the right edge of the window, with the button still down: the
    // shell reports the move and nothing else.
    frame(&mut state, vec![egui::Event::PointerMoved(egui::pos2(1400.0, 200.0))]);
    assert!(
        ctx.dragged_id().is_some(),
        "the bar let go of the drag when the pointer left the window",
    );
    assert_eq!(
        state.spectrum_config.smoothing, 0.9,
        "the bar stopped following the pointer at the window edge (it reads {inside} still)",
    );

    // And the release, delivered outside the window, ends it — the case the
    // shell used to drop on the floor (egui-baseview sent a button event only
    // when it had a pointer position, and the exit had cleared it).
    frame(&mut state, vec![press(egui::pos2(1400.0, 200.0), false)]);
    assert!(
        ctx.dragged_id().is_none(),
        "a release outside the window left the drag standing, which strands every pane's wheel",
    );
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

/// The Commas section reads as a table: one row per comma, and the same two
/// columns in the same order down every row — the temperament switch and its
/// auto-detect.
///
/// Positions rather than presence, because a table whose cells do not line up
/// is exactly the failure a "does it draw the word Marvel" test would pass.
/// The Auto column is located by its heading: its switches are bare (the
/// heading is their label), so there is no text in the cells to find.
///
/// The comma each row tempers out is deliberately NOT drawn — it lives in the
/// switch's hover, where a ratio is read once rather than kept in a column
/// that every row has to make room for.
#[test]
fn the_commas_section_lays_its_rows_out_as_a_table() {
    let shapes = settings_pane_at_width(
        panes::Tab::Tuning,
        423.0,
        projections_for(panes::Tab::Tuning)[0],
    );
    let find = |needle: &str| {
        shapes.iter().find_map(|cs| match &cs.shape {
            egui::Shape::Text(t) if t.galley.text() == needle => Some(t.pos),
            _ => None,
        })
    };
    let at = |needle: &str| {
        find(needle).unwrap_or_else(|| panic!("the Tuning pane drew no {needle:?}"))
    };
    let (meantone, marvel) = (at("Meantone"), at("Marvel"));
    let (temper_head, auto_head) = (at("Temper"), at("Auto"));

    // Rows: the commas run down the table in `Comma::ALL` order.
    assert!(marvel.y > meantone.y, "the rows are out of order");
    // Columns: the two rows' switches share a left edge, and Auto is right of
    // the names rather than under them.
    assert!((meantone.x - marvel.x).abs() < 1.0, "the name column is ragged");
    assert!(meantone.x < auto_head.x, "the name column is not leftmost");
    // Headings sit above the first row, one over each column.
    for head in [temper_head, auto_head] {
        assert!(head.y < meantone.y, "a heading is not above the rows");
    }
    assert!(temper_head.x < auto_head.x);
    // And the ratios are on the hover, not in the table.
    for ratio in ["81/80", "225/224"] {
        assert!(find(ratio).is_none(), "{ratio} is drawn in the table");
    }
}
