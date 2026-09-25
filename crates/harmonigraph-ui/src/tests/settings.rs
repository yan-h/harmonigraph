//! The settings panes as a column: that each scrolls when it overflows, that
//! the bar it scrolls by covers nothing, that no control overruns a narrow one,
//! and the Video pane's own rows.

use super::harness::*;
use crate::*;

#[test]
fn opening_analyzer_settings_does_not_change_loaded_values() {
    let mut state = fresh();
    state.picture.appearance.spectrum.tilt = -2.0;
    assert!(state.load_persist(&state.save_persist()));
    assert_eq!(state.picture.appearance.spectrum.tilt, -1.5);
    let before = ron::to_string(&state.picture.appearance.spectrum).unwrap();
    let tab = panes::Tab::AnalyzerSettings;
    state.workspace.layout.select(tab);
    let mut harness = DockHarness::at(egui::vec2(1000.0, 1600.0));
    let output = harness.frame(&mut state, vec![]);
    assert!(
        output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Tilt (dB/oct)")
        }),
        "the no-input frame must reach the tilt controls"
    );
    assert_eq!(ron::to_string(&state.picture.appearance.spectrum).unwrap(), before);
}

/// Drive the REAL dock (root_ui, the workspace, the tab body's ScrollArea and
/// all) with a wheel over `tab`'s body, and answer how far its content moved.
/// Negative = the content moved up, i.e. the pane scrolled down.
///
/// Tracks NAMED texts rather than a bounding box: egui culls whatever scrolls
/// out of the clip rect and the custom bars paint past it, so every
/// position-of-the-ink metric reports movement that isn't there (and misses
/// movement that is). The y of a string drawn in both frames cannot lie.
fn wheel_over_settings_pane(pane: panes::Tab, screen_h: f32) -> f32 {
    let mut state = fresh();
    // The settings leaf opens on Tuning; every other settings pane is a tab
    // behind it.
    state.workspace.layout.select(pane);
    let mut h = DockHarness::at(egui::vec2(1000.0, screen_h));
    // The top-right leaf (right of the 0.72 split, above the 0.55 one), from
    // under its tab bar down. Only shapes clipped to this are the pane's.
    let body = egui::Rect::from_min_max(egui::pos2(700.0, 20.0), egui::pos2(1000.0, screen_h));
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
    let mut frame =
        |state: &mut SharedState, events: Vec<egui::Event>| texts(&h.frame(state, events));
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
    let mut deltas: Vec<f32> =
        before.iter().filter_map(|(text, y)| after.get(text).map(|moved| moved - y)).collect();
    assert!(!deltas.is_empty(), "{pane:?} drew no text to measure");
    deltas.sort_by(f32::total_cmp);
    deltas[deltas.len() / 2]
}

/// Every settings pane scrolls to the wheel once its content is taller than
/// the pane. All of them reach the wheel through the `ScrollArea` the workspace
/// wraps each tab body in, which is what leaves the bar the pane's right margin
/// to stand in (see [`nothing_is_drawn_under_a_settings_pane_scroll_bar`]).
///
/// The two readout panes build an area of their own and are not swept here: the
/// Console sticks to the bottom, where a wheel DOWN is a no-op, so it answers
/// this question with the opposite sign and needs a fixture of its own rather
/// than a row in this one.
#[test]
fn every_settings_pane_scrolls_when_its_content_overflows() {
    // A short window, so that every one of them overflows — including the
    // System page, the shortest list of the set.
    for pane in [
        panes::Tab::Tuning,
        panes::Tab::Colors,
        panes::Tab::LatticeSettings,
        panes::Tab::AnalyzerSettings,
        panes::Tab::System,
        panes::Tab::Video,
    ] {
        let moved = wheel_over_settings_pane(pane, 200.0);
        assert!(moved < -8.0, "{pane:?} did not scroll to the wheel (content moved {moved})");
    }
}

/// The Note section's Fade curve bar draws the curve the NOTES run on, not a
/// second copy of the formula that happens to look like it.
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
        panes::Tab::LatticeSettings,
        320.0,
        harmonigraph_scene::Projection::default(),
    )
    .into_iter()
    .map(|cs| cs.shape)
    .collect();
    // The one RISING preview: the glow's and the shadows' falloffs on the same
    // page all descend.
    let paths = crate::widgets::curve_paths(&shapes);
    let rising: Vec<_> =
        paths.iter().filter(|path| path.first().unwrap().y > path.last().unwrap().y).collect();
    assert_eq!(rising.len(), 1, "the Lattice page drew {} rising curve previews", rising.len());
    let points = rising[0];
    assert!(points.len() > 8, "the Fade curve bar drew {} preview points", points.len());

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
    for point in points {
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

/// The Glow section draws the same falloff `ViewConfig` hands to the scene.
/// The bar is the setting's readout, so a line drifting from the renderer
/// is a false value just as surely as a bar printing the wrong number.
#[test]
fn the_glow_curve_bar_draws_the_curve_the_scene_receives() {
    let shapes: Vec<egui::Shape> = settings_pane_at_width(
        panes::Tab::LatticeSettings,
        320.0,
        harmonigraph_scene::Projection::default(),
    )
    .into_iter()
    .map(|cs| cs.shape)
    .collect();
    let paths = crate::widgets::curve_paths(&shapes);
    let mut descending: Vec<&Vec<egui::Pos2>> =
        paths.iter().filter(|path| path.first().unwrap().y < path.last().unwrap().y).collect();
    // The glow's and the enabled Distance falloffs of the lattice's own two
    // shadow groups. Disabled Gaussian bars are dimmed out of curve_paths'
    // identifying color. Count rather than silently selecting past an
    // unexpected curve.
    let shadow = harmonigraph_scene::ShadowSettings::default();
    assert_eq!(
        descending.len(),
        1 + [shadow.lattice_geometry, shadow.lattice_text]
            .iter()
            .filter(|style| style.kernel.is_distance())
            .count(),
        "the Lattice page drew {} descending curves",
        descending.len()
    );
    // The topmost is the glow's: its section stands above the Shadows on the
    // page, which is a fact about the layout rather than about draw order.
    descending.sort_by(|a, b| a[0].y.total_cmp(&b[0].y));
    let points = descending[0];
    assert!(points.len() > 8, "the glow curve used only {} points", points.len());

    let curve = harmonigraph_scene::ViewConfig::default().glow_curve;
    let (left, right) = (points[0].x, points[points.len() - 1].x);
    let (top, bottom) = (points[0].y, points[points.len() - 1].y);
    assert!(left < right && top < bottom, "the glow's falloff is not descending");
    for point in points {
        let p = (point.x - left) / (right - left);
        let want = top + (bottom - top) * (1.0 - curve.sample(p));
        assert!(
            (point.y - want).abs() < 0.02,
            "at {p} across the reach the bar is at {} and the scene curve at {want}",
            point.y,
        );
    }
}

/// The Video pane drawn through the REAL dock, soloed, for a shell that can or
/// cannot record takes — the one thing that changes which section leads it.
///
/// Through `root_ui` and the workspace rather than calling `Viewer::ui` on a
/// hand-built child, because the wrapping is the part under test: the workspace
/// puts every body inside a `ScrollArea`, and that ui arrives with a
/// full-height `min_rect` where a hand-built one arrives empty. A fixture that
/// skips it cannot see the difference — see `section`.
fn video_pane_shapes(supported: bool) -> (Vec<egui::epaint::ClippedShape>, egui::Color32) {
    let mut state = fresh();
    state.workspace.interaction.take.supported = supported;
    // Soloed so the Video pane's body is the only settings body on screen and
    // the first heading found is unambiguously its own.
    state.workspace.layout = workspace::Layout::solo(panes::Tab::Video);
    // Tall and narrow: one soloed pane, with room for its whole column.
    let mut h = DockHarness::at(egui::vec2(420.0, 1200.0));
    let out = h.frame(&mut state, vec![]);
    // What `ui.separator()` strokes with, so the assertion names the rule
    // rather than "some line" — the tab bar draws its own, in its own colors.
    let rule = h.ctx.style_of(egui::Theme::Dark).visuals.widgets.noninteractive.bg_stroke.color;
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
/// `record_controls` returns early and Frame leads instead. `section` decides
/// it from what has been drawn rather than from the caller, and this is what
/// holds it to that for the case the caller could not have known.
#[test]
fn the_video_pane_does_not_start_with_a_rule() {
    // Which section leads, per shell: a host can record takes, so Record
    // leads; the standalone cannot, so `record_controls` returns early.
    for (supported, leads) in [(true, "RECORD"), (false, "FRAME")] {
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

/// A folded section's heading sits midway between the rule over it and the
/// rule that opens the next section, with its chevron ahead of the name.
///
/// The space that sets a section off from the one above once went in above
/// the rule alone, which is the same space landing UNDER a folded heading and
/// nowhere over it. "Audio analysis" because a section with one below it is
/// the only kind with a rule on both sides to measure between.
#[test]
fn a_folded_heading_sits_centred_between_its_rules() {
    let mut state = fresh();
    state.workspace.layout.select(panes::Tab::AnalyzerSettings);
    state.workspace.interaction.folded_sections.insert("Analyzer/Audio analysis".to_owned());
    let mut h = DockHarness::at(egui::vec2(1000.0, 1600.0));
    h.settle(&mut state);
    let out = h.frame(&mut state, vec![]);
    let leaf = state.workspace.layout_runtime.rects[workspace::Section::Settings as usize];
    let rule = h.ctx.style_of(egui::Theme::Dark).visuals.widgets.noninteractive.bg_stroke.color;
    let heading = out
        .shapes
        .iter()
        .find_map(|cs| match &cs.shape {
            egui::Shape::Text(t) if t.galley.text() == "AUDIO ANALYSIS" && leaf.contains(t.pos) => {
                Some(egui::Rect::from_min_size(t.pos, t.galley.size()))
            }
            _ => None,
        })
        .expect("the Analyzer page drew no Audio analysis heading");
    let rules: Vec<f32> = out
        .shapes
        .iter()
        .filter_map(|cs| match &cs.shape {
            egui::Shape::LineSegment { points, stroke }
                if stroke.color == rule
                    && points[0].y == points[1].y
                    && leaf.contains(points[0]) =>
            {
                Some(points[0].y)
            }
            _ => None,
        })
        .collect();
    let above = rules.iter().copied().filter(|&y| y < heading.top()).fold(f32::MIN, f32::max);
    let below = rules.iter().copied().filter(|&y| y > heading.bottom()).fold(f32::MAX, f32::min);
    assert!(above > f32::MIN && below < f32::MAX, "no rule on both sides of {heading:?}");
    let (over, under) = (heading.top() - above, below - heading.bottom());
    assert!((over - under).abs() < 1.0, "{over}pt over the folded heading, {under}pt under it");
    assert!(
        out.shapes.iter().any(|cs| matches!(&cs.shape,
            egui::Shape::Path(p) if p.points.len() == 3 && {
                let mark = egui::Rect::from_points(&p.points);
                mark.right() < heading.left() && (mark.center().y - heading.center().y).abs() < 1.0
            }
        )),
        "no chevron ahead of the folded heading"
    );
}

/// The standalone keeps the one render control it can act on, and neither shell
/// grows a row the other should not have.
///
/// `render_controls` serves both shells out of one function, and which rows each
/// gets is decided by where a single `if !supported { return }` sits: above the
/// Spectrogram row the standalone loses it, below the row it keeps it. That is a
/// line of ordering doing the work of a policy, and nothing else in the suite
/// looks at it — hoisting that return two lines leaves all 439 tests green while
/// the standalone silently loses its only say over what an offline render bakes,
/// `RenderConfig::spectrogram` becoming unreachable from that shell entirely.
///
/// Both directions, because a gate is two claims: the Spectrogram row survives
/// with no take support, and the rows that need a take to mean anything do not
/// appear without one. `the_video_pane_does_not_start_with_a_rule` above is the
/// only other test that draws this pane in the standalone, and it asks about the
/// FIRST heading, so every row below it is unwatched.
#[test]
fn the_standalone_keeps_the_render_row_a_take_is_not_needed_for() {
    // Shared by both shells: the section, the row, and the choice on it. The
    // standalone has no transport to record with and still renders, so this is
    // the one thing in Render it can act on.
    for row in ["RENDER", "Video history", "Whole video"] {
        for supported in [true, false] {
            let (shapes, _) = video_pane_shapes(supported);
            assert!(
                text_y(&shapes, row).is_some(),
                "take.supported={supported}: the Video pane drew no {row:?}",
            );
        }
    }
    // Take-only, and gated on exactly the same flag: a shell that cannot record
    // has nothing for this to describe.
    let row = "Finish recording";
    let (with, _) = video_pane_shapes(true);
    assert!(text_y(&with, row).is_some(), "a recording shell drew no {row:?}");
    let (without, _) = video_pane_shapes(false);
    assert!(
        text_y(&without, row).is_none(),
        "the standalone drew {row:?}, which needs a take to mean anything",
    );
}

/// Scrolling names the span it scrolls, read live off the Analyzer's History
/// duration, so it reads as the alternative to Whole video without the Analyzer
/// page open. Dialled off the default so a label frozen at the fresh value fails.
#[test]
fn the_scrolling_spectrogram_choice_names_its_span() {
    let mut state = fresh();
    state.picture.appearance.spectrum.roll_seconds = 42.0;
    state.workspace.layout = workspace::Layout::solo(panes::Tab::Video);
    let shapes = DockHarness::at(egui::vec2(420.0, 1200.0)).frame(&mut state, vec![]).shapes;
    assert!(text_y(&shapes, "Scrolling (42.0 s)").is_some(), "Scrolling did not name its span");
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
        for &pane in SETTINGS_PANES {
            for &projection in projections_for(pane) {
                let widths = bar_track_widths(&settings_pane_at_width(pane, width, projection));
                // One bar per gradient is deliberately shorter: the spectrum
                // track, which gives the right end of its row to the flip
                // button. It still narrows with the column, which is what this
                // is about, so it is allowed its own length rather than excused
                // from the sweep.
                //
                // COUNTED, not merely permitted. A sweep that accepts either
                // length from any bar accepts a spectrum track that never
                // reserved the button's width at all — it comes out at the
                // column's own length and passes on the first alternative,
                // with the button painted over its left end. So the count is
                // exact: TWO on the Colors page, which carries both gradients
                // — the lattice's pitch table and the heatmap's level table, on
                // the same three bars over the same type — and none anywhere
                // else.
                let track = crate::widgets::spectrum_track_width(width, 1.0);
                let mut short = 0;
                for bar in &widths {
                    if (bar - width).abs() < 1.0 {
                        continue;
                    }
                    short += 1;
                    assert!(
                        (bar - track).abs() < 1.0,
                        "{pane:?}/{projection:?} at {width}pt drew a {bar}pt bar, \
                         neither the column nor the spectrum track's {track}pt \
                         (all of {widths:?})"
                    );
                }
                let want = if pane == panes::Tab::Colors { 2 } else { 0 };
                assert_eq!(
                    short, want,
                    "{pane:?}/{projection:?} at {width}pt drew {short} short bars, not {want} \
                     (all of {widths:?})"
                );
            }
        }
    }
    // The sniffing above finds nothing if the bars stop being painted this way,
    // and a test that measures nothing passes. The Lattice page is the deepest
    // stack of bars in the dock — every layer of the note contributing one or
    // more, on top of the camera and the depth axis — so it is the pane to ask
    // whether bars are still being painted at all.
    //
    // The floor is a count of the bars a fresh view draws LIVE, and a gated bar
    // is not in it: [`bar_track_widths`] finds a track by the well's own fill,
    // and a disabled `Ui` fades its painter, so a greyed track is no longer
    // that color. Sheet size is inert with Sheets at its fresh 0, and
    // half the node layers gate on something — hence a floor well under the
    // count. What the floor watches for is the paint going away, which takes
    // every bar at once; a control coming, going or greying is not what it is
    // asking about.
    let bars = bar_track_widths(&settings_pane_at_width(
        panes::Tab::LatticeSettings,
        400.0,
        PROJECTIONS[0],
    ))
    .len();
    assert!(bars >= 12, "only found {bars} bar tracks on the Lattice page; has the paint changed?");
}

/// Each settings tab draws its own body, and only that body: a string only
/// that tab draws is there, and each other tab's is not.
///
/// A tab wired to a neighbour's arm draws the wrong body under the right name,
/// which nothing else here would notice: the sweeps ask each body for
/// properties — a bar is the column's width, a bar is one row high, the pane
/// scrolls — that hold whatever the body contains, so two tabs could trade
/// bodies with the suite green. The needles also pin where the old Lighting
/// page's settings went: the lattice's glow with the Lattice, the ribbons'
/// bloom with the Analyzer.
#[test]
fn each_settings_tab_draws_its_own_body_and_only_that() {
    const CASES: [(panes::Tab, &str); 4] = [
        (panes::Tab::Colors, "Pitch color range"),
        (panes::Tab::LatticeSettings, "Glow reach"),
        (panes::Tab::AnalyzerSettings, "Ribbon bloom"),
        (panes::Tab::System, "Lattice resolution"),
    ];
    for (tab, needle) in CASES {
        let mut state = fresh();
        let shapes = tab_body(&mut state, tab, 400.0, PANE_HEIGHT).shapes;
        let drawn = |text: &str| {
            shapes.iter().any(|cs| match &cs.shape {
                egui::Shape::Text(t) => t.galley.text() == text,
                _ => false,
            })
        };
        assert!(drawn(needle), "{tab:?} drew no {needle:?} — it is showing another tab's body");
        for (other, stranger) in CASES {
            if other != tab {
                assert!(!drawn(stranger), "{tab:?} drew {stranger:?}, which is {other:?}'s");
            }
        }
    }
}

/// Each gradient group opens with its preview: one band of color the width of
/// the column, standing ABOVE the spectrum track that is the first of the three
/// bars writing it.
///
/// The order is the whole claim. Nothing else here would notice it moving — the
/// preview paints no well, so the width and height sweeps both step over it,
/// and a group that drew its picture at the bottom, or dropped it, would leave
/// every other test green.
///
/// Both groups, in the one page that now carries them: the note gradient's
/// leads, the heatmap's follows behind its preset row, and they are separate
/// calls over the same widgets. Which is also why each preview is read against
/// its OWN gradient — two groups on one page is exactly the arrangement in
/// which handing one of them the other's table draws a picture that is wrong
/// about everything and wrong in no position.
#[test]
fn every_gradient_group_previews_itself_above_its_bars() {
    const WIDTH: f32 = 400.0;
    let shapes = settings_pane_at_width(panes::Tab::Colors, WIDTH, PROJECTIONS[0]);
    // A preview is a full-column band of color: a spectrum's circle is the
    // track's width and a fade ramp is a row high, so the pair of measurements
    // tells all three apart.
    let mut drawn: Vec<&egui::Mesh> = shapes
        .iter()
        .filter_map(|cs| match &cs.shape {
            egui::Shape::Mesh(m) => Some(&**m),
            _ => None,
        })
        .filter(|m| {
            let b = crate::widgets::band_bounds(m);
            (b.width() - WIDTH).abs() < 1.0
                && (b.height() - crate::widgets::preview_height(1.0)).abs() < 0.6
        })
        .collect();
    assert_eq!(drawn.len(), 2, "the Colors page drew {} gradient previews, not two", drawn.len());
    // Down the page, which is the order the groups are written in: note colors
    // first, heatmap colors under them.
    drawn.sort_by(|a, b| {
        crate::widgets::band_bounds(a).top().total_cmp(&crate::widgets::band_bounds(b).top())
    });

    let track = crate::widgets::spectrum_track_width(WIDTH, 1.0);
    let mut tracks: Vec<egui::Rect> = shapes
        .iter()
        .filter_map(|cs| match &cs.shape {
            egui::Shape::Rect(r)
                if r.fill == crate::theme::well() && (r.rect.width() - track).abs() < 1.0 =>
            {
                Some(r.rect)
            }
            _ => None,
        })
        .collect();
    assert_eq!(tracks.len(), 2, "the Colors page drew {tracks:?} as spectrum tracks, not two");
    tracks.sort_by(|a, b| a.top().total_cmp(&b.top()));

    for (group, mesh, bar, gradient) in [
        (
            "note colors",
            drawn[0],
            tracks[0],
            harmonigraph_scene::ViewConfig::default().pitch_gradient,
        ),
        ("heatmap colors", drawn[1], tracks[1], SpectrumConfig::default().spectrogram_gradient),
    ] {
        // Read at the ends, where the ramp is the table's own first and last
        // entry and no interpolation stands between the mesh and the value.
        let lut = harmonigraph_scene::color::pitch_ramp_lut(gradient.sanitized());
        let columns: Vec<egui::Color32> =
            crate::widgets::band_columns(mesh).into_iter().map(|(_, _, color)| color).collect();
        for (end, drew, want) in
            [("low", columns[0], lut[0]), ("high", columns[columns.len() - 1], lut[lut.len() - 1])]
        {
            let want = crate::panes::scene_color(want, 1.0);
            assert_eq!(
                drew, want,
                "the {group} group drew its preview's {end} end in {drew:?}, not the \
                 {want:?} its own gradient reaches — the other group's?",
            );
        }
        let preview = crate::widgets::band_bounds(mesh);
        assert!(
            preview.bottom() <= bar.top(),
            "the {group} group drew its preview at {preview:?}, not above the \
             spectrum bar at {bar:?}",
        );
    }
}

/// The render bar fills to the share of frames done — which is the whole
/// reason it is a bar and not another sentence in the status line, since a
/// render is minutes long and the sentence never changes while it runs.
#[test]
fn the_render_bar_fills_to_the_share_of_frames_done() {
    const WIDTH: f32 = 400.0;
    let shapes = settings_pane_at_width(panes::Tab::Video, WIDTH, PROJECTIONS[0]);
    let share = FIXTURE_RENDER.fraction().expect("the fixture render knows its total");
    // A polygon rather than a rect: a fill is the part of its track left of
    // the frontier (`filled_part`), so what is measured is the reach of the
    // shape the bar actually painted. Its box is a full row tall wherever the
    // frontier is clear of the track's own corner, as it is in this fixture.
    let fills: Vec<f32> = shapes
        .iter()
        .filter_map(|cs| match &cs.shape {
            egui::Shape::Path(path) if path.fill == crate::theme::accent_fill() => {
                let box_of = egui::Rect::from_points(&path.points);
                ((box_of.height() - crate::theme::ROW_HEIGHT).abs() < 0.6)
                    .then(|| box_of.width() / WIDTH)
            }
            _ => None,
        })
        .collect();
    assert!(
        fills.iter().any(|filled| (filled - share).abs() < 0.01),
        "no bar filled to {share} of the column; found {fills:?}"
    );
}

/// The cancel stands with the render bar and nowhere else, and pressing it
/// asks the shell to stop that render.
///
/// Both halves in one fixture because they are one claim: a button offering to
/// stop a render that is not running, and a button that stops nothing, are the
/// two ways this reads as working without being. Through the REAL dock, so the
/// press arrives the way a pointer's does — a `clicked()` that no layer between
/// the mouse and the pane delivers is exactly the failure a hand-built ui
/// cannot see.
#[test]
fn the_cancel_stands_with_the_render_bar_and_asks_for_the_stop() {
    let mut state = fresh();
    state.workspace.interaction.take.supported = true;
    state.workspace.interaction.take.last_ready = true;
    // Soloed and tall, like `video_pane_shapes`: the whole control column on
    // screen, so a button that is missing is missing rather than scrolled off.
    state.workspace.layout = workspace::Layout::solo(panes::Tab::Video);
    let mut h = DockHarness::at(egui::vec2(420.0, 1200.0));
    let find = |shapes: &[egui::epaint::ClippedShape]| {
        shapes.iter().find_map(|cs| match &cs.shape {
            egui::Shape::Text(t) if t.galley.text() == "Cancel render" => Some(t.pos),
            _ => None,
        })
    };

    // The pane's steady state: a take to re-render, and nothing rendering.
    let idle = h.frame(&mut state, vec![]).shapes;
    assert!(find(&idle).is_none(), "a cancel drawn with no render to cancel");

    state.workspace.interaction.take.render_progress = Some(FIXTURE_RENDER);
    // Two frames: egui resolves the widget under the pointer from the previous
    // pass, so the button has to have been drawn before the press.
    h.frame(&mut state, vec![]);
    let running = h.frame(&mut state, vec![]).shapes;
    let text = find(&running).expect("no cancel while a render is running");
    // Inside the button rather than at the text's own corner, which sits on
    // the frame's inner edge.
    let at = text + egui::vec2(4.0, 4.0);

    h.frame(&mut state, vec![egui::Event::PointerMoved(at)]);
    h.frame(&mut state, vec![egui::Event::PointerMoved(at), press(at, true)]);
    h.frame(&mut state, vec![press(at, false)]);
    assert!(state.workspace.interaction.take.cancel_render, "the press never reached the shell");
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
        let panes = SETTINGS_PANES
            .iter()
            .copied()
            .flat_map(|pane| projections_for(pane).iter().map(move |&p| (pane, p)));
        for (pane, projection) in panes {
            let shapes = settings_pane_at_width(pane, width, projection);
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
                "{pane:?}/{projection:?} at {width}pt ran {:?} past the pane edge",
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
///
/// [`Lose::Nothing`] is the case with nothing for the first two arms of
/// `end_stranded_drag` to read: focus kept, pointer inside, and the release
/// simply never delivered. It is the one a person meets as "the settings pane
/// stopped scrolling and clicking back into the window does not fix it", so it
/// is the wheel itself that has to end the drag.
#[test]
fn a_drag_that_loses_its_release_does_not_strand_the_wheel() {
    // Default dock: the Analyzer picture is the column at x ~518..720, the
    // settings leaf is top-right. The bar is grabbed by NAME — where it sits
    // under the Display pane's headers is layout, not this test's business.
    for (what, grab) in [
        ("the analyzer picture", Grab::Point(egui::pos2(600.0, 200.0))),
        ("a settings bar", Grab::Bar("Frequency range")),
    ] {
        for lose_it in [Lose::Pointer, Lose::Focus, Lose::Nothing] {
            let moved = scroll_settings_after_lost_drag(grab, lose_it).0;
            assert!(
                moved < -8.0,
                "a drag on {what} that lost its release to {lose_it:?} left the settings \
                 pane unscrollable (content moved {moved})",
            );
        }
    }
}

/// A drag the wheel had to step over says so in the Console.
///
/// The wheel arm fires only where the shell's own repair has already failed —
/// the OS was asked whether the button is held and the release was still never
/// synthesised — so it is the one arm whose firing is evidence rather than
/// routine. Without the line the symptom repairs itself silently and the cause
/// stays unreachable; the other two arms stay quiet, because a gesture let go
/// outside the window ends that way every time.
#[test]
fn the_console_names_a_drag_the_wheel_had_to_end() {
    let (_, logged) = scroll_settings_after_lost_drag(Grab::Bar("Frequency range"), Lose::Nothing);
    assert!(
        logged.iter().any(|line| line.starts_with("wheel: a drag on")),
        "the wheel ended a stranded drag without saying so: {logged:?}",
    );
    for quiet in [Lose::Pointer, Lose::Focus] {
        let (_, logged) = scroll_settings_after_lost_drag(Grab::Bar("Frequency range"), quiet);
        assert!(
            !logged.iter().any(|line| line.starts_with("wheel:")),
            "{quiet:?} is an ordinary end of a gesture and reported one: {logged:?}",
        );
    }
}

/// How the release goes missing: the pointer leaves the editor, the host takes
/// focus while the button is down, or nothing at all happens and the release is
/// merely never delivered.
#[derive(Clone, Copy, Debug)]
enum Lose {
    Pointer,
    Focus,
    Nothing,
}

/// Where the doomed drag takes hold: a fixed point in a picture pane, or a
/// named settings bar found where it was painted — a bar draws its name at
/// its own left end, so a small offset from the text is inside the track.
#[derive(Clone, Copy, Debug)]
enum Grab {
    Point(egui::Pos2),
    Bar(&'static str),
}

/// Press and drag at `grab`, lose the release, then wheel over the settings
/// pane and answer how far its content moved, with whatever the Console was
/// told along the way.
fn scroll_settings_after_lost_drag(grab: Grab, lose: Lose) -> (f32, Vec<String>) {
    let mut state = fresh();
    // The Analyzer settings.
    let tab = panes::Tab::AnalyzerSettings;
    state.workspace.layout.select(tab);
    // Tall enough that the Analyzer's first bars are inside the settings leaf:
    // a bar this fixture presses on outside the leaf is a press on the pane
    // below.
    let screen_h = 360.0;
    let mut h = DockHarness::at(egui::vec2(1000.0, screen_h));
    let body = egui::Rect::from_min_max(egui::pos2(700.0, 20.0), egui::pos2(1000.0, screen_h));
    // Named texts inside the settings body, as `wheel_over_settings_pane` does:
    // the position of a string drawn in both frames is the one metric a clip
    // rect and a culled shape cannot lie about. The whole position rather than
    // the y alone, so [`Grab::Bar`] can aim a press at a bar it can name.
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
                map.entry(t.galley.text().to_owned()).or_insert(t.pos);
            }
        }
        map
    };
    // A handle on the harness's own context, taken before the closure below
    // borrows it: `egui::Context` is an `Arc`, so this reads the same context
    // the frames run on rather than a second one.
    let ctx = h.ctx.clone();
    let mut frame =
        |state: &mut SharedState, events: Vec<egui::Event>| texts(&h.frame(state, events));
    // Where the press lands, off a laid-out frame: a named bar is wherever
    // the layout put it, which under the Display pane's headers is nowhere a
    // constant can point.
    let laid_out = frame(&mut state, vec![]);
    let from = match grab {
        Grab::Point(pos) => pos,
        Grab::Bar(name) => {
            let pos = laid_out
                .get(name)
                .unwrap_or_else(|| panic!("no {name:?} bar in the settings body"));
            *pos + egui::vec2(2.0, 4.0)
        }
    };
    // Hover, press, drag — and then the release goes missing.
    frame(&mut state, vec![egui::Event::PointerMoved(from)]);
    frame(&mut state, vec![press(from, true)]);
    frame(&mut state, vec![egui::Event::PointerMoved(from + egui::vec2(0.0, 40.0))]);
    // The gesture must have taken hold, or losing it costs nothing and the
    // wheel below was never in danger — a fixture aimed at a label would pass
    // whatever the stranding did.
    assert!(ctx.dragged_id().is_some(), "the press for {grab:?} started no drag");
    frame(
        &mut state,
        match lose {
            Lose::Pointer => vec![egui::Event::PointerGone],
            Lose::Focus => vec![egui::Event::WindowFocused(false)],
            Lose::Nothing => vec![],
        },
    );

    // Wheel over the settings gutter, clear of controls that consume scroll.
    let settings = egui::pos2(995.0, 130.0);
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
        before.iter().filter_map(|(text, pos)| after.get(text).map(|m| m.y - pos.y)).collect();
    assert!(!deltas.is_empty(), "the settings pane drew no text to measure");
    deltas.sort_by(f32::total_cmp);
    let moved = deltas[deltas.len() / 2];
    (moved, state.picture.runtime.console.lines().map(str::to_owned).collect())
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
    let mut state = fresh();
    // The Analyzer settings.
    let tab = panes::Tab::AnalyzerSettings;
    state.workspace.layout.select(tab);
    // Tall enough that Release is actually on screen below the view,
    // analysis and level-mapping controls; a clipped bar cannot start this drag.
    let screen_h = 1800.0;
    let mut h = DockHarness::at(egui::vec2(1000.0, screen_h));
    // The settings leaf, whose bars run the width of the column at x ~700..1000:
    // from under its tab bar down to the 0.55 split.
    let body = egui::Rect::from_min_max(egui::pos2(700.0, 20.0), egui::pos2(1000.0, screen_h));
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
    // As above: an `Arc` handle on the harness's own context, so the drag
    // state read below is the one the frames wrote.
    let ctx = h.ctx.clone();
    let mut frame = |state: &mut SharedState, events: Vec<egui::Event>| h.frame(state, events);
    // Release: a plain 0..=0.5 bar (`settings::BALLISTICS_MAX`), so where the
    // pointer is says what the value should be, and the far end of the range is
    // what an off-window drag to the right must arrive at.
    let out = frame(&mut state, vec![]);
    let name =
        bar_named(&out, "Live release").expect("the Release bar is drawn on the Analyzer page");
    let on_the_bar = name + egui::vec2(2.0, 4.0);
    let before = state.picture.appearance.spectrum.release;
    frame(&mut state, vec![egui::Event::PointerMoved(on_the_bar)]);
    frame(&mut state, vec![press(on_the_bar, true)]);
    frame(&mut state, vec![egui::Event::PointerMoved(on_the_bar + egui::vec2(60.0, 0.0))]);
    assert!(ctx.dragged_id().is_some(), "the press on the Release bar started no drag");
    let inside = state.picture.appearance.spectrum.release;
    assert!(inside != before, "the bar did not follow the pointer inside the window");

    // Out past the right edge of the window, with the button still down: the
    // shell reports the move and nothing else.
    frame(&mut state, vec![egui::Event::PointerMoved(egui::pos2(1400.0, 200.0))]);
    assert!(
        ctx.dragged_id().is_some(),
        "the bar let go of the drag when the pointer left the window",
    );
    assert_eq!(
        state.picture.appearance.spectrum.release, 0.5,
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
    let moved = wheel_over_settings_pane(panes::Tab::Video, 330.0);
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
#[test]
fn the_commas_section_lays_its_rows_out_as_a_table() {
    let shapes = settings_pane_at_width(panes::Tab::Tuning, 423.0, PROJECTIONS[0]);
    let find = |needle: &str| {
        shapes.iter().find_map(|cs| match &cs.shape {
            egui::Shape::Text(t) if t.galley.text() == needle => Some(t.pos),
            _ => None,
        })
    };
    let at =
        |needle: &str| find(needle).unwrap_or_else(|| panic!("the Tuning pane drew no {needle:?}"));
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
}

/// A policy bar sends one configuration command on every changed drag frame,
/// and the audio thread can adopt each one before the next frame arrives. The
/// pending notice therefore comes and goes DURING one gesture. It must not sit
/// above the bar it describes: moving the bar under a stationary pointer makes
/// the next frame write a different value, which brings the notice back and
/// makes the whole Adaptive tuning section flicker between two positions.
#[test]
fn a_transient_tuning_status_does_not_move_the_adaptive_controls() {
    let adaptive_y = |pending| {
        let mut state = fresh();
        state.picture.runtime.configuration_pending = pending;
        let output = tab_body(&mut state, panes::Tab::Tuning, 423.0, PANE_HEIGHT);
        if pending {
            assert!(
                text_y(&output.shapes, "Tuning change pending audio adoption").is_some(),
                "the pending fixture never drew the transient status",
            );
        }
        one_text_y(&output.shapes, "ADAPTIVE TUNING")
    };

    let (settled, pending) = (adaptive_y(false), adaptive_y(true));
    assert_eq!(
        pending, settled,
        "the transient status moved Adaptive tuning from {settled} to {pending}",
    );
}

/// The size a settings pane is soloed at to make it scroll: narrow enough that
/// the bars run the width of the column, and short enough that every pane in
/// the sweep — System, the shortest list of them — overflows it.
const SCROLLING_PANE: egui::Vec2 = egui::vec2(320.0, 200.0);

/// The chrome scales the sweep runs at: the design size, both ends of
/// [`theme::UI_SCALE_RANGE`], and two in between that leave `8 * scale`
/// fractional — which is the case the gutter truncates and the bar must not
/// (see `theme::dock_pane_margin`).
const SCALES: [f32; 5] = [0.7, 0.9, 1.0, 1.1, 1.5];

/// One settings pane soloed in the REAL dock at a size it overflows, as the
/// shapes it drew.
///
/// The dock and not [`settings_pane_at_width`], because where a scroll bar goes
/// is a question about the wrapping: the workspace puts each body in a `ScrollArea`
/// of its own, and a fixture that calls `Viewer::ui` on a hand-built child
/// draws the panes that rely on it with no bar at all.
///
/// The pointer rests inside the pane because a bar nobody is pointing at is
/// drawn at zero opacity, and a fully transparent rect answers with no visual
/// bounding rect — so an unhovered bar is not in the shapes to find. Hovering
/// the AREA is enough, and hovering the BAR is not asked for: the panes keep
/// their bars in two different places, no one pointer is over both, and the lane
/// is the bar's full width in from its right edge however thin it is painted.
///
/// Both readout panes list what has come in, and an empty one has nothing to
/// scroll, so the fixture gives the Console lines and the Notes pane voices.
fn scrolling_settings_pane(
    pane: panes::Tab,
    scale: f32,
) -> (Vec<egui::epaint::ClippedShape>, egui::Rect) {
    let mut state = fresh();
    let tab = pane;
    state.workspace.layout = workspace::Layout::solo(tab);
    // The same shell [`settings_pane_at_width`] draws for, so the Video pane
    // brings its record row and its progress bar — the two controls
    // `widgets::bar_width` calls out as having nowhere to wrap to, and so the
    // two likeliest to reach the lane.
    state.workspace.interaction.take.supported = true;
    state.workspace.interaction.take.last_ready = true;
    state.workspace.interaction.take.render_progress = Some(FIXTURE_RENDER);
    for i in 0..40 {
        state
            .picture
            .runtime
            .console
            .log(format!("{i:02} a log line long enough to run the width of the pane"));
    }
    for note in 40..80 {
        state.picture.runtime.tracker.handle_event(harmonigraph_core::NoteEvent::on(
            0.5,
            harmonigraph_core::SourceId::DIRECT,
            0,
            note,
            1.0,
        ));
    }
    // The window scales with the chrome, so every pane overflows it by the same
    // margin at every scale rather than the sweep having a size per scale.
    let mut h = DockHarness::scaled(SCROLLING_PANE * scale, scale, &mut state);
    let screen = h.screen;
    let mut frame = |state: &mut SharedState, events: Vec<egui::Event>| h.frame(state, events);
    let margin = crate::theme::pane_inner_margin(scale);
    let inside = egui::pos2(screen.right() - 2.0 * margin, screen.center().y);
    frame(&mut state, vec![egui::Event::PointerMoved(inside)]);
    let mut out = frame(&mut state, vec![]);
    // The widening is animated, so the bar reaches its full width a few frames
    // after the pointer arrives.
    for _ in 0..20 {
        out = frame(&mut state, vec![]);
    }
    (out.shapes, pane_body(&state, &tab).expect("settings body is visible"))
}

/// The theme's widest scroll bar at a given chrome scale, read back off a
/// themed context rather than restated here.
fn scroll_bar_width(scale: f32) -> f32 {
    super::probe::themed_scaled(scale).style_of(egui::Theme::Dark).spacing.scroll.bar_width
}

/// Nothing a settings pane draws goes under its own scroll bar.
///
/// A floating bar is painted OVER the content instead of beside it, so the only
/// place it fits without covering a control is air the pane already leaves: the
/// [`theme::PANE_INNER_MARGIN`] gutter down the right of the tab body. Two
/// things put it there, and neither is visible from the pane that depends on
/// it — the bar is sized to the gutter (`theme::style_at`), and a pane that
/// scrolls in an area of its OWN starts at the content box, where there is no
/// gutter, so it reserves one (`theme::reserve_scroll_gutter`).
///
/// Get either wrong and the bar lands on the right end of every row in the
/// column, which is where the value readouts are. A pane that builds its own
/// area and does not reserve is the loud version: its bar stands a whole margin
/// in from the pane edge with every point of its width over the controls, where
/// an oversized bar only overhangs them by the difference.
///
/// The lane is FOUND rather than assumed, because where it lands is the thing
/// under test. What identifies it is the bar's own rect: no wider than a bar,
/// and as tall as the area it scrolls, which is not a shape a pane draws.
#[test]
fn nothing_is_drawn_under_a_settings_pane_scroll_bar() {
    for scale in SCALES {
        let margin = f32::from(crate::theme::dock_pane_margin(scale));
        let bar = scroll_bar_width(scale);
        assert!(
            bar <= margin + 0.01,
            "at {scale} a {bar}pt bar does not fit the {margin}pt gutter"
        );
        for &pane in SETTINGS_PANES {
            let (shapes, body) = scrolling_settings_pane(pane, scale);
            // The pane's own shapes are the ones clipped to the tab BODY. The dock's
            // chrome — the leaf fill, the body border, the tab bar and its rule — is
            // clipped to the leaf, which starts a tab bar higher up.
            let pane_shape = |cs: &egui::epaint::ClippedShape| {
                let rect = cs.shape.visual_bounding_rect();
                let mine = cs.clip_rect.top() >= body.top() - 0.5;
                // Shapes that carry no geometry answer with an inverted or infinite
                // rect; egui's own `is_finite` lets those through.
                (mine && rect.is_finite() && rect.width() < 1.0e4).then_some(rect)
            };
            let bar_edges: Vec<f32> = shapes
                .iter()
                .filter_map(|cs| {
                    let rect = pane_shape(cs)?;
                    let is_rect = matches!(cs.shape, egui::Shape::Rect(_));
                    (is_rect && rect.width() <= bar + 0.5 && rect.height() >= body.height() * 0.5)
                        .then(|| rect.right())
                })
                .collect();
            assert!(
                !bar_edges.is_empty(),
                "{pane:?} at scale {scale} drew no scroll bar, so it never overflowed and this \
             proves nothing about it",
            );
            // The lane is the bar's full width in from its right edge, whatever the
            // bar is painted at: a dormant one is a `floating_width` hairline, but
            // egui senses the whole `bar_width` either way, so a control under the
            // thin end of it is as unreachable as one under the fat end.
            let right = bar_edges.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let leftmost = bar_edges.iter().copied().fold(f32::INFINITY, f32::min);
            assert!(
            right - leftmost < 0.5,
            "{pane:?} at {scale} drew scroll bars in two places, ending at {leftmost} and {right}",
        );
            let left = right - bar;

            // Where the lane lands: against the pane edge for a pane the dock
            // scrolls, whose area IS the body. The Console scrolls in an area of
            // its own — the Clear row above it must not scroll — so its lane
            // stands one content margin further in.
            let own_area = matches!(pane, panes::Tab::Console);
            let edge = body.right() - if own_area { margin } else { 0.0 };
            assert!(
                (right - edge).abs() < 0.5,
                "{pane:?} at {scale} puts its scroll bar at {left}..{right}, not against {edge}",
            );

            let under: Vec<String> = shapes
                .iter()
                .filter_map(|cs| {
                    let rect = pane_shape(cs)?;
                    if rect.right() <= left + 1.0 {
                        return None;
                    }
                    // The bar itself: a background rect and a handle, both inside
                    // the lane they define.
                    if matches!(cs.shape, egui::Shape::Rect(_))
                        && rect.left() >= left - 0.5
                        && rect.right() <= right + 0.5
                    {
                        return None;
                    }
                    // The scroll area's own fade at the foot of its content, which
                    // is the width of the AREA. Every control in a pane starts at
                    // the content box, a margin in, so nothing outside it is one.
                    if rect.left() < body.left() + margin - 0.5 {
                        return None;
                    }
                    Some(match &cs.shape {
                        egui::Shape::Text(t) => format!("{:?}", t.galley.text()),
                        other => format!("{other:?}").chars().take(60).collect(),
                    })
                })
                .collect();
            assert!(
                under.is_empty(),
                "{pane:?} at {scale} draws into its scroll bar's lane {left}..{right}: {under:?}",
            );
        }
    }
}

/// The Tuning pane's comma table keeps its cells clear of the bar that scrolls
/// them sideways.
///
/// The other half of [`theme::reserve_scroll_gutter`], and the half no pane
/// margin can cover: this area scrolls HORIZONTALLY, so its bar runs under the
/// table rather than down a side, and a row of cells has no gutter along its
/// bottom for one to float in. Unreserved, the lane lands 5.5pt up inside the
/// bottom row.
///
/// Its own test because the table only overflows in a column dragged under
/// about 135pt (see `comma_controls`), and at every width that fits the table
/// there is no sideways bar for the sweep above to find.
///
/// The pointer is parked on the Temper heading — inside the area, so the bar is
/// awake and in the shapes at all, and on a plain label rather than a switch,
/// whose hover would put a second copy of a row's name on screen to be found
/// instead of the cell.
#[test]
fn the_comma_tables_sideways_bar_runs_under_its_cells() {
    let mut state = fresh();
    state.workspace.layout = workspace::Layout::solo(panes::Tab::Tuning);
    // Narrower than the two columns need, and tall enough that the pane does
    // not also scroll — one bar in the picture is one bar to find.
    let rails = 2.0 * (crate::theme::tab_bar_height(1.0) + 3.0);
    let mut h = DockHarness::at(egui::vec2(120.0 + rails, 900.0));
    h.settle(&mut state);
    let screen = state.workspace.layout_runtime.rects[workspace::Section::Settings as usize];
    let mut frame = |state: &mut SharedState, events: Vec<egui::Event>| h.frame(state, events);
    let heading_rect = |out: &egui::FullOutput| {
        out.shapes.iter().find_map(|cs| match &cs.shape {
            egui::Shape::Text(text) if text.galley.text() == "Temper" => {
                Some(cs.shape.visual_bounding_rect())
            }
            _ => None,
        })
    };
    // Where the table is has to be read off a frame before the pointer can be
    // put in it: the Commas section sits wherever the sections above it end.
    let out = frame(&mut state, vec![]);
    let heading = heading_rect(&out).expect("the Tuning pane drew no Temper heading");
    let mut out = frame(&mut state, vec![egui::Event::PointerMoved(heading.center())]);
    for _ in 0..20 {
        out = frame(&mut state, vec![]);
    }

    let bar = f32::from(crate::theme::dock_pane_margin(1.0));
    let pane = |cs: &egui::epaint::ClippedShape| {
        let rect = cs.shape.visual_bounding_rect();
        (rect.is_finite() && rect.width() < 1.0e4).then_some(rect)
    };
    // The bar: as thin as a bar and most of the pane wide, below the table's
    // headings — not the shape of anything in a two-column table of switches.
    let painted = out
        .shapes
        .iter()
        .filter(|cs| matches!(cs.shape, egui::Shape::Rect(_)))
        .filter_map(pane)
        .filter(|rect| {
            rect.height() <= bar + 0.5
                && rect.width() >= screen.width() * 0.5
                && rect.top() > heading.bottom()
        })
        .fold(egui::Rect::NOTHING, egui::Rect::union);
    assert!(
        painted.is_finite(),
        "the comma table drew no sideways bar at {}pt, so it never overflowed and this proves \
         nothing about it",
        screen.width(),
    );
    // Painted at `floating_width` — the pointer is in the area, not on the bar —
    // so the lane is a full bar up from its bottom edge, which is what egui
    // senses and what a hover fills in.
    let lane_top = painted.bottom() - bar;

    // The table's own cells are the shapes the area clips to its content box.
    // The bar and the area's fade are clipped to the whole tab body, which is a
    // margin wider on each side, so neither is mistaken for one.
    let cells = out
        .shapes
        .iter()
        .filter(|cs| cs.clip_rect.width() < screen.width() - 1.0)
        .filter_map(pane)
        .filter(|rect| {
            rect.top() >= heading.top() - 0.5 && rect.bottom() <= painted.bottom() + 0.5
        });
    let lowest = cells.fold(f32::NEG_INFINITY, |low, rect| low.max(rect.bottom()));
    assert!(
        lowest > heading.bottom(),
        "the table drew its headings and no rows under them, so there is nothing to measure",
    );
    assert!(
        lowest <= lane_top + 0.5,
        "the table's cells run to {lowest} and the lane of the bar under them starts at {lane_top}",
    );
}

/// The Lattice page drawn with the audio ring `width` thick and carrying
/// `reading`, as the shapes it emitted.
fn audio_section_shapes(
    reading: harmonigraph_scene::SpectralReading,
    width: f32,
) -> Vec<egui::epaint::ClippedShape> {
    let mut state = fresh();
    state.picture.appearance.view.spectral_reading = reading;
    state.picture.appearance.view.spectral_ring_width = width;
    // A middle with room for the ring switched on above. The fresh middle is a
    // look and free to grow, and a ring refused for room greys every bar that
    // sizes it, which reads here as a gate that never opens.
    state.picture.appearance.view.ring_inner = 0.3;
    let tab = panes::Tab::LatticeSettings;
    tab_body(&mut state, tab, 320.0, PANE_HEIGHT).shapes
}

/// The y every run of `needle` in `shapes` was painted at.
fn text_ys(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Vec<f32> {
    shapes
        .iter()
        .filter_map(|cs| match &cs.shape {
            egui::Shape::Text(t) if t.galley.text() == needle => Some(t.pos.y),
            _ => None,
        })
        .collect()
}

/// The y of the one run of `needle`, for the names painted once in this pane.
fn one_text_y(shapes: &[egui::epaint::ClippedShape], needle: &str) -> f32 {
    let ys = text_ys(shapes, needle);
    assert_eq!(ys.len(), 1, "{needle:?} is painted {} times, not once", ys.len());
    ys[0]
}

/// The colour of the TRACK behind the bar whose name sits at `y` — the widest
/// rect that row is drawn on.
///
/// The track and not the name, though the name is what a reader looks at: a bar
/// lays its label out in an explicit `theme::text_dim()`, and `Ui::disable`
/// greys by tinting the colours a SHAPE carries, which for a galley with a
/// colour of its own is the fallback nothing reads. The track is a plain rect,
/// so the tint lands on it — which is also why a greyed bar reads as greyed on
/// screen at all.
fn track_color(shapes: &[egui::epaint::ClippedShape], y: f32) -> egui::Color32 {
    shapes
        .iter()
        .filter_map(|cs| match &cs.shape {
            egui::Shape::Rect(r) if r.rect.y_range().contains(y) => Some((r.rect.width(), r.fill)),
            _ => None,
        })
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .unwrap_or_else(|| panic!("no bar track is drawn on the row at y {y}"))
        .1
}

/// The spectrogram keeps its time axis when MIDI is hidden; ribbon-only
/// controls are visibly disabled, so no live bar silently does nothing.
#[test]
fn history_stays_editable_without_midi_ribbons() {
    let colors = |show_roll| {
        let mut state = fresh();
        state.picture.appearance.spectrum.show_roll = show_roll;
        state.picture.appearance.spectrum.show_spectrogram = true;
        let tab = panes::Tab::AnalyzerSettings;
        let shapes = tab_body(&mut state, tab, 420.0, PANE_HEIGHT).shapes;
        ["History duration", "Ribbon width", "Ribbon opacity", "Extension release"]
            .map(|name| track_color(&shapes, one_text_y(&shapes, name)))
    };
    let shown = colors(true);
    let hidden = colors(false);
    assert_eq!(shown[0], hidden[0], "the spectrogram lost its history control");
    assert_ne!(shown[1], hidden[1], "ribbon width stayed live without ribbons");
    assert_ne!(shown[2], hidden[2], "ribbon opacity stayed live without ribbons");
    assert_ne!(shown[3], hidden[3], "extension release stayed live without ribbons");
}

/// Each reading's own bar is the LIVE one — Tolerance under Fold, Zoom under
/// Spectrum. The entire group is absent with the ring itself, which is sized on
/// the Layers bar two sections up.
///
/// Nothing else in the tree looks at these gates. They are two `add_enabled_ui`
/// predicates twenty lines apart that each name the other's enum variant, which
/// is exactly the shape a swap survives: exchange them and the pane ships with
/// its only draggable bar the one that does nothing, while every other test
/// here stays green.
///
/// The ring dialled to nothing is the case worth naming, and it is why the
/// section keeps no off switch of its own: the layer is switched off from its
/// handle, its settings disappear, and what turns it back on is a control this
/// section does not hold.
///
/// Gate is live under BOTH readings, which is the one bar here that is: it says
/// which nodes wear the ring rather than how either reading is measured, so a
/// version that tied it to one of them would leave the other's rings ungateable
/// while every claim below still held.
///
/// Read off the PAINTED track rather than a response flag, because a gate egui
/// honours and a gate the pane merely believes in look the same from inside the
/// pane — the tint is the painter's, so the colour is the one reading that has
/// been through it.
#[test]
fn each_readings_own_bar_is_the_one_that_is_live() {
    use harmonigraph_scene::SpectralReading;

    let row = |(reading, width): (SpectralReading, f32), name: &str| {
        let shapes = audio_section_shapes(reading, width);
        let reading_row = one_text_y(&shapes, "Ring display");
        let zoom = one_text_y(&shapes, "Pitch span");
        let y = match name {
            // Taken by POSITION and not by paint order, so the row measured is
            // provably the Audio section's: a Tolerance drawn anywhere but
            // between the Reading row and Zoom fails here rather than being read
            // as this section's bar.
            "Pitch tolerance" => text_ys(&shapes, "Pitch tolerance")
                .into_iter()
                .find(|y| *y > reading_row && *y < zoom)
                .expect("the Audio section's Tolerance bar sits between Reading and Zoom"),
            "Pitch span" => zoom,
            // Between the Reading row and Zoom, by position for the same
            // reason: it is the section's own bar or this measures nothing.
            "Ring threshold" => text_ys(&shapes, "Ring threshold")
                .into_iter()
                .find(|y| *y > reading_row && *y < zoom)
                .expect("the Audio section's Gate bar sits between Reading and Zoom"),
            other => panic!("{other:?} is not a bar in the Audio section"),
        };
        track_color(&shapes, y)
    };

    const WIDE: f32 = 0.3;
    let live = row((SpectralReading::Fold, WIDE), "Pitch tolerance");
    let dead = row((SpectralReading::Fold, WIDE), "Pitch span");
    assert_ne!(live, dead, "a bar's track paints the same greyed and live, so nothing below bites");

    for (state, want_width, want_range, want_gate) in [
        ((SpectralReading::Fold, WIDE), live, dead, live),
        ((SpectralReading::Spectrum, WIDE), dead, live, live),
    ] {
        assert_eq!(
            row(state, "Pitch tolerance"),
            want_width,
            "{state:?}: Tolerance is the wrong way"
        );
        assert_eq!(row(state, "Pitch span"), want_range, "{state:?}: Zoom is the wrong way");
        assert_eq!(row(state, "Ring threshold"), want_gate, "{state:?}: Gate is the wrong way");
    }
}

/// With no audio layer in the picture, its heading remains as the signpost but
/// its settings spend no vertical room. Giving the layer any drawable width
/// reveals the complete group again.
#[test]
fn audio_ring_settings_hide_with_the_layer() {
    use harmonigraph_scene::SpectralReading;

    for reading in [SpectralReading::Fold, SpectralReading::Spectrum] {
        let hidden = audio_section_shapes(reading, 0.0);
        assert_eq!(text_ys(&hidden, "AUDIO RING").len(), 1, "the section lost its heading");
        for setting in [
            "Ring display",
            "Ring threshold",
            "Threshold hysteresis",
            "Ring attack",
            "Ring release",
            "Pitch tolerance",
            "Pitch span",
        ] {
            assert!(
                text_ys(&hidden, setting).is_empty(),
                "{setting:?} remained visible with {reading:?} dialled off",
            );
        }

        let expanded = audio_section_shapes(reading, 0.3);
        for setting in [
            "Ring display",
            "Ring threshold",
            "Threshold hysteresis",
            "Ring attack",
            "Ring release",
            "Pitch tolerance",
            "Pitch span",
        ] {
            assert_eq!(
                text_ys(&expanded, setting).len(),
                1,
                "{setting:?} did not return with {reading:?} enabled",
            );
        }
    }
}

/// Falloff bends the distance field used by Contour shadows. Blur supplies its
/// own Gaussian profile, so the four groups keep their common width and depth
/// controls there without showing a falloff setting that cannot affect them.
#[test]
fn shadow_falloff_only_appears_for_contour_shadows() {
    use harmonigraph_scene::ShadowKernel;

    // Both pages that hold shadow groups, two groups each.
    let shapes = |kernel| {
        let mut state = fresh();
        for style in state.picture.appearance.view.shadow.groups_mut() {
            style.kernel = kernel;
        }
        [panes::Tab::LatticeSettings, panes::Tab::AnalyzerSettings]
            .into_iter()
            .flat_map(|tab| tab_body(&mut state, tab, 420.0, PANE_HEIGHT).shapes)
            .collect::<Vec<_>>()
    };

    let blurred = shapes(ShadowKernel::Gaussian);
    assert!(text_ys(&blurred, "Shadow falloff").is_empty());
    assert_eq!(text_ys(&blurred, "Shadow width").len(), 4, "Blur lost common shadow controls");

    let contour = shapes(ShadowKernel::Distance);
    assert_eq!(
        text_ys(&contour, "Shadow falloff").len(),
        4,
        "a Contour shadow group has no falloff control",
    );
    assert_eq!(text_ys(&contour, "Shadow width").len(), 4, "Contour lost common shadow controls",);
}

/// No settings tab draws two sections under one heading. A fold is saved as
/// "Tab/Heading", so two headings of one name on one tab would fold together.
///
/// Headings are told from the rest of the text by their letter spacing, the one
/// thing nothing else in the panel sets, and the window is tall enough that
/// every tab's whole list is laid out at once. Sections a fresh state hides
/// (Note retuning, and Record outside a host) are not seen.
#[test]
fn no_settings_tab_repeats_a_section_heading() {
    for &tab in workspace::Section::Settings.tabs() {
        let mut state = fresh();
        state.workspace.layout.select(tab);
        let mut window = DockHarness::at(egui::vec2(1000.0, 8000.0));
        window.settle(&mut state);
        let out = window.frame(&mut state, vec![]);
        let mut titles: Vec<String> = out
            .shapes
            .iter()
            .filter_map(|cs| match &cs.shape {
                egui::Shape::Text(text)
                    if text
                        .galley
                        .job
                        .sections
                        .iter()
                        .all(|s| s.format.extra_letter_spacing > 0.0) =>
                {
                    Some(text.galley.text().to_owned())
                }
                _ => None,
            })
            .collect();
        let drawn = titles.len();
        titles.sort();
        titles.dedup();
        assert_eq!(titles.len(), drawn, "{tab:?} repeats a heading: {titles:?}");
        if !matches!(tab, panes::Tab::Console) {
            assert!(drawn >= 2, "{tab:?} drew {drawn} headings; are they still letter-spaced?");
        }
    }
}

/// Every section heading on every settings tab sits `GROUP_GAP` from the rule
/// over it and from the first thing drawn under it, measured ink to ink on
/// what the frame actually painted — whatever kind of row that first thing is.
///
/// It holds only while every row kind keeps its box where its ink is: text
/// through `widgets::label`, fold headers and switches trimmed to their
/// capitals, bars and buttons their fill. A settings line drawn with a bare
/// `ui.label` sits its leading lower and fails here, which is what this is
/// for. The tolerance is the ascenders and brackets that stand a point above
/// a line's capitals.
#[test]
fn every_section_heading_stands_one_gap_from_its_neighbours() {
    fn ink_tops(shape: &egui::Shape, out: &mut Vec<(egui::Rect, f32)>) {
        let visible = |c: egui::Color32| c.a() > 0;
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| ink_tops(s, out)),
            egui::Shape::Text(t) if !t.galley.is_empty() => {
                let ink = t.galley.mesh_bounds.translate(t.pos.to_vec2());
                out.push((ink, ink.top()));
            }
            egui::Shape::Rect(r) if visible(r.fill) || visible(r.stroke.color) => {
                out.push((r.rect, r.rect.top()))
            }
            egui::Shape::LineSegment { points, stroke } if visible(stroke.color) => {
                let r = egui::Rect::from_two_pos(points[0], points[1]);
                out.push((r, r.top()));
            }
            egui::Shape::Path(p) if !p.points.is_empty() => {
                let r = egui::Rect::from_points(&p.points);
                out.push((r, r.top()));
            }
            egui::Shape::Circle(c) if visible(c.fill) => {
                let r = egui::Rect::from_center_size(c.center, egui::Vec2::splat(2.0 * c.radius));
                out.push((r, r.top()));
            }
            egui::Shape::Mesh(m) if !m.vertices.is_empty() => {
                let r = m.calc_bounds();
                out.push((r, r.top()));
            }
            _ => {}
        }
    }

    let gap = crate::widgets::GROUP_GAP;
    let mut misses = Vec::new();
    let mut checked = 0;
    for &tab in workspace::Section::Settings.tabs() {
        let mut state = fresh();
        state.workspace.layout.select(tab);
        let mut window = DockHarness::at(egui::vec2(1000.0, 8000.0));
        window.settle(&mut state);
        let out = window.frame(&mut state, vec![]);
        let leaf = state.workspace.layout_runtime.rects[workspace::Section::Settings as usize];
        let mut inks = Vec::new();
        for cs in &out.shapes {
            ink_tops(&cs.shape, &mut inks);
        }
        inks.retain(|(rect, _)| leaf.intersects(*rect) && rect.top() > leaf.top());
        let rule = h_rule_color(&window);
        for cs in &out.shapes {
            let egui::Shape::Text(t) = &cs.shape else { continue };
            let spaced = t.galley.job.sections.iter().all(|s| s.format.extra_letter_spacing > 0.0);
            if !spaced || !leaf.contains(t.pos) {
                continue;
            }
            let (Some(row), name) = (t.galley.rows.first(), t.galley.text()) else { continue };
            let Some(glyph) = row.glyphs.first() else { continue };
            let baseline = t.pos.y + row.pos.y + glyph.pos.y;
            let caps = t.pos.y + t.galley.mesh_bounds.top();
            // Over: the nearest rule above, when there is one (the first
            // section of a tab has none).
            let over = out
                .shapes
                .iter()
                .filter_map(|cs| match &cs.shape {
                    egui::Shape::LineSegment { points, stroke }
                        if stroke.color == rule
                            && points[0].y == points[1].y
                            && points[0].y < caps
                            && leaf.contains(points[0]) =>
                    {
                        Some(points[0].y)
                    }
                    _ => None,
                })
                .fold(f32::MIN, f32::max);
            if over > caps - 3.0 * gap {
                let measured = caps - over;
                if (measured - gap).abs() > 1.0 {
                    misses.push(format!("{tab:?} {name}: {measured:.1}pt from the rule over it"));
                }
            }
            // Under: the highest ink below the baseline that is not the
            // heading's own chevron beside it.
            let under = inks
                .iter()
                .filter(|(rect, top)| *top > baseline + 0.5 && rect.left() >= leaf.left())
                .map(|(_, top)| *top)
                .fold(f32::MAX, f32::min);
            if under < baseline + 3.0 * gap {
                let measured = under - baseline;
                if (measured - gap).abs() > 1.5 {
                    misses.push(format!("{tab:?} {name}: {measured:.1}pt to what is under it"));
                }
            }
            checked += 1;
        }
    }
    assert!(checked >= 20, "only {checked} headings measured; are they still letter-spaced?");
    assert!(misses.is_empty(), "headings off their gap:\n{}", misses.join("\n"));
}

/// The colour a section rule is stroked in.
fn h_rule_color(window: &DockHarness) -> egui::Color32 {
    window.ctx.style_of(egui::Theme::Dark).visuals.widgets.noninteractive.bg_stroke.color
}
