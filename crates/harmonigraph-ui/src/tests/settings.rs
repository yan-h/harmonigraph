//! The settings panes as a column: that each scrolls when it overflows, that
//! the bar it scrolls by covers nothing, that no control overruns a narrow one,
//! and the Video pane's own rows.

use crate::*;
use super::harness::*;

/// Put the Notes/Console leaf back on screen, which is what the two wheel
/// harnesses below are written against: they read the settings leaf as the box
/// from the tab bar down to the 0.55 split, and the default layout opens that
/// leaf folded (see
/// `the_default_layout_opens_with_the_two_readout_panes_folded`, in
/// `tests/fold.rs`) so the
/// settings column runs the whole height instead.
///
/// Unfolded rather than measured where it now is, because a taller pane is the
/// wrong pane to ask these questions of: both tests need content that
/// OVERFLOWS, and the short window they pick is short relative to this box.
fn unfold_the_readout_panes(state: &mut SharedState) {
    let path = state.workspace.dock.find_tab(&panes::Tab::Notes).expect("Notes is docked");
    state.workspace.dock[path.surface][path.node].set_collapsed(false);
}

/// Drive the REAL dock (root_ui, egui_dock, the tab body's ScrollArea and
/// all) with a wheel over `tab`'s body, and answer how far its content moved.
/// Negative = the content moved up, i.e. the pane scrolled down.
///
/// Tracks NAMED texts rather than a bounding box: egui culls whatever scrolls
/// out of the clip rect and the custom bars paint past it, so every
/// position-of-the-ink metric reports movement that isn't there (and misses
/// movement that is). The y of a string drawn in both frames cannot lie.
fn wheel_over_settings_pane(pane: SettingsPane, screen_h: f32) -> f32 {
    let mut state = fresh();
    unfold_the_readout_panes(&mut state);
    // The settings leaf opens on Tuning; every other settings pane is a tab
    // behind it (a Section is the Display tab with that section open).
    let tab = pane.install(&mut state);
    let path = state.workspace.dock.find_tab(&tab).expect("{tab:?} is not in the default dock");
    state.workspace.dock.set_active_tab(path).expect("selecting the tab");
    let mut h = DockHarness::at(egui::vec2(1000.0, screen_h));
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
    let mut deltas: Vec<f32> = before
        .iter()
        .filter_map(|(text, y)| after.get(text).map(|moved| moved - y))
        .collect();
    assert!(!deltas.is_empty(), "{pane:?} drew no text to measure");
    deltas.sort_by(f32::total_cmp);
    deltas[deltas.len() / 2]
}

/// Every settings pane scrolls to the wheel once its content is taller than
/// the pane. All nine reach the wheel through the `ScrollArea` egui_dock wraps
/// each tab body in, which is what leaves the bar the pane's right margin to
/// stand in (see [`nothing_is_drawn_under_a_settings_pane_scroll_bar`]).
/// Display is swept once per section, opened alone: a section's content plus
/// the five headers standing over it is what overflows.
///
/// The two readout panes build an area of their own and are not swept here: the
/// Console sticks to the bottom, where a wheel DOWN is a no-op, so it answers
/// this question with the opposite sign and needs a fixture of its own rather
/// than a row in this one.
#[test]
fn every_settings_pane_scrolls_when_its_content_overflows() {
    // A short window, so that every one of them overflows — including System,
    // the shortest list of the set.
    for pane in [
        SettingsPane::Tab(panes::Tab::Tuning),
        SettingsPane::Section(Section::Color),
        SettingsPane::Section(Section::View),
        SettingsPane::Section(Section::Nodes),
        SettingsPane::Section(Section::Labels),
        SettingsPane::Section(Section::Grid),
        SettingsPane::Section(Section::Analyzer),
        SettingsPane::Tab(panes::Tab::Video),
        SettingsPane::Tab(panes::Tab::System),
    ] {
        let moved = wheel_over_settings_pane(pane, 300.0);
        assert!(moved < -8.0, "{pane:?} did not scroll to the wheel (content moved {moved})");
    }
}

/// The Nodes section's Fade curve bar draws the curve the NOTES run on, not a
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
        SettingsPane::Section(Section::Nodes),
        320.0,
        harmonigraph_scene::Projection::default(),
    )
    .into_iter()
    .map(|cs| cs.shape)
    .collect();
    let points = crate::widgets::curve_points(&shapes);
    assert!(points.len() > 8, "the Nodes section drew {} preview points", points.len());

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
    let mut state = fresh();
    state.take.supported = supported;
    // Soloed so the Video pane's body is the only settings body on screen and
    // the first heading found is unambiguously its own.
    state.workspace.dock = egui_dock::DockState::new(vec![panes::Tab::Video]);
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

/// The standalone keeps the one render control it can act on, and neither shell
/// grows a row the other should not have.
///
/// `render_controls` serves both shells out of one function, and which rows each
/// gets is decided by where a single `if !supported { return }` sits: above the
/// Spectrogram row the standalone loses it, below the row it keeps it. That is a
/// line of ordering doing the work of a policy, and nothing else in the suite
/// looks at it — hoisting that return two lines leaves all 439 tests green while
/// the standalone silently loses its only say over what an offline render bakes,
/// `RenderConfig::playhead` becoming unreachable from that shell entirely.
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
    for row in ["Render", "Spectrogram", "Live", "Playhead"] {
        for supported in [true, false] {
            let (shapes, _) = video_pane_shapes(supported);
            assert!(
                text_y(&shapes, row).is_some(),
                "take.supported={supported}: the Video pane drew no {row:?}",
            );
        }
    }
    // Take-only, and gated on exactly the same flag: a shell that cannot record
    // has nothing for these to describe.
    for row in ["Finish", "Lead-in"] {
        let (with, _) = video_pane_shapes(true);
        assert!(text_y(&with, row).is_some(), "a recording shell drew no {row:?}");
        let (without, _) = video_pane_shapes(false);
        assert!(
            text_y(&without, row).is_none(),
            "the standalone drew {row:?}, which needs a take to mean anything",
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
        for pane in SETTINGS_PANES {
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
                // with the button painted over its left end. So the short
                // length is allowed exactly once per pane that holds one, and
                // two do: the Color & light section dials the lattice's pitch
                // gradient and the Analyzer section the heatmap's level
                // gradient, on the same three bars over the same type.
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
                let want = usize::from(matches!(
                    pane,
                    SettingsPane::Section(Section::Color | Section::Analyzer)
                ));
                assert_eq!(
                    short, want,
                    "{pane:?}/{projection:?} at {width}pt drew {short} short bars, not {want} \
                     (all of {widths:?})"
                );
            }
        }
    }
    // The sniffing above finds nothing if the bars stop being painted this way,
    // and a test that measures nothing passes. The Nodes section is the deepest
    // stack of bars in the dock — 17 against the Analyzer's 12 and the View's
    // 8, every layer of the note contributing one or more — so it is the pane
    // to ask whether bars are still being painted at all.
    //
    // Those three are counts of the bars a fresh view draws LIVE, and a gated
    // bar is in none of them: [`bar_track_widths`] finds a track by the well's
    // own fill, and a disabled `Ui` fades its painter, so a greyed track is no
    // longer that color. Hence the View's 8 rather than 9 — Sevenths size is
    // inert at the fresh sevenths extent of 0, and counting it needs an extent
    // set — and hence a floor well under 17. What the floor watches for is the
    // paint going away, which takes every bar at once; a control coming, going
    // or greying is not what it is asking about.
    let bars = bar_track_widths(&settings_pane_at_width(
        SettingsPane::Section(Section::Nodes),
        400.0,
        PROJECTIONS[0],
    ))
    .len();
    assert!(
        bars >= 12,
        "only found {bars} bar tracks in the Nodes section; has the paint changed?"
    );
}

/// The bool named for a section opens that section's body, and only that one.
///
/// [`DisplaySections`] is one field per section and `open_mut` is the map from
/// a section to its own, so a section wired to a neighbour's field draws the
/// wrong body under the right header. Nothing else here would notice: the
/// sweeps ask each body for properties — a bar is the column's width, a bar is
/// one row high, the pane scrolls — that hold whatever the body contains, so
/// two sections could trade bodies with the suite green. Labels and Grid are
/// the pair that would go longest unnoticed, being two bars each and named in
/// no other test.
///
/// **The field is set by NAME, never through `open_mut`**, and that is the
/// whole reach of this test. A fixture that opens a section the way
/// [`SettingsPane::install`] does reads the map to write the flag and reads it
/// again to draw, so a wrong arm cancels itself out and the fixture passes on
/// exactly the defect it was written for.
///
/// A text out of each BODY rather than the header over it. The six headers are
/// drawn whatever is open — that is what a collapsed section is — so they say
/// nothing about which body a flag reached. Each needle is a string only its
/// own section draws: "Name size" would be the natural one for Labels and is
/// not, the Analyzer's piano-roll group having a bar of that name too.
#[test]
fn the_field_named_for_a_section_opens_that_sections_body() {
    let draw = |sections: crate::panes::display::DisplaySections| {
        let mut state = fresh();
        state.display_sections = sections;
        tab_body(&mut state, panes::Tab::Display, 400.0, PANE_HEIGHT).shapes
    };
    type Open = fn(&mut crate::panes::display::DisplaySections);
    const CASES: [(Open, &str, &str); 6] = [
        (|d| d.color = true, "color", "Bloom"),
        (|d| d.view = true, "view", "Projection"),
        (|d| d.nodes = true, "nodes", "Solidity"),
        (|d| d.labels = true, "labels", "Keep note names"),
        (|d| d.grid = true, "grid", "Line width"),
        (|d| d.analyzer = true, "analyzer", "Smoothing"),
    ];
    for (open, field, needle) in CASES {
        let mut sections = crate::panes::display::DisplaySections::default();
        open(&mut sections);
        let shapes = draw(sections);
        let drawn = |text: &str| {
            shapes.iter().any(|cs| match &cs.shape {
                egui::Shape::Text(t) => t.galley.text() == text,
                _ => false,
            })
        };
        assert!(drawn(needle), "`{field}` set drew no {needle:?} — it opened no body");
        for (_, other, stranger) in CASES {
            if other != field {
                assert!(
                    !drawn(stranger),
                    "`{field}` set drew {stranger:?}, which is `{other}`'s — \
                     are the two wired to each other's fields?",
                );
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
/// every other test green. Both panes are asked, because the two groups are
/// separate calls over the same widgets and only one of them has the preset row
/// above (see `spectrogram_gradient_group`).
#[test]
fn every_gradient_group_previews_itself_above_its_bars() {
    const WIDTH: f32 = 400.0;
    for section in [Section::Color, Section::Analyzer] {
        let shapes = settings_pane_at_width(SettingsPane::Section(section), WIDTH, PROJECTIONS[0]);
        // A preview is the one full-column band of color in a settings pane: a
        // spectrum's circle is the track's width and a fade ramp is a row high,
        // so the pair of measurements tells all three apart.
        let drawn: Vec<&egui::Mesh> = shapes
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
        assert_eq!(drawn.len(), 1, "{section:?} drew {} gradient previews, not one", drawn.len());
        let previews: Vec<egui::Rect> =
            drawn.iter().map(|m| crate::widgets::band_bounds(m)).collect();

        // And it is THIS section's gradient. The two sections dial different
        // gradients through the same widgets, so a group handed the other one
        // draws a picture that is wrong about everything and wrong in no
        // position — every other assertion here passes on it. Read at the ends,
        // where the ramp is the table's own first and last entry and no
        // interpolation stands between the mesh and the value.
        let gradient = match section {
            Section::Color => harmonigraph_scene::ViewConfig::default().pitch_gradient,
            _ => SpectrumConfig::default().spectrogram_gradient,
        };
        let lut = harmonigraph_scene::color::pitch_ramp_lut(gradient.sanitized());
        let columns: Vec<egui::Color32> = crate::widgets::band_columns(drawn[0])
            .into_iter()
            .map(|(_, _, color)| color)
            .collect();
        for (end, drew, want) in [
            ("quiet", columns[0], lut[0]),
            ("loud", columns[columns.len() - 1], lut[lut.len() - 1]),
        ] {
            let want = crate::panes::scene_color(want, 1.0);
            assert_eq!(
                drew, want,
                "{section:?} drew its preview's {end} end in {drew:?}, not the \
                 {want:?} its own gradient reaches — the other section's?",
            );
        }
        let track = crate::widgets::spectrum_track_width(WIDTH, 1.0);
        let tracks: Vec<egui::Rect> = shapes
            .iter()
            .filter_map(|cs| match &cs.shape {
                egui::Shape::Rect(r)
                    if r.fill == crate::theme::well()
                        && (r.rect.width() - track).abs() < 1.0 =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect();
        assert_eq!(tracks.len(), 1, "{section:?} drew {tracks:?} as spectrum tracks, not one");
        assert!(
            previews[0].bottom() <= tracks[0].top(),
            "{section:?} drew its preview at {:?}, not above the spectrum bar at {:?}",
            previews[0],
            tracks[0],
        );
    }
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
    let shapes =
        settings_pane_at_width(SettingsPane::Tab(panes::Tab::Video), WIDTH, PROJECTIONS[0]);
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
        let panes = SETTINGS_PANES
            .into_iter()
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
#[test]
fn a_drag_that_loses_its_release_does_not_strand_the_wheel() {
    // Default dock: the Analyzer picture is the column at x ~518..720, the
    // settings leaf is top-right. The bar is grabbed by NAME — where it sits
    // under the Display pane's headers is layout, not this test's business.
    for (what, grab) in [
        ("the analyzer picture", Grab::Point(egui::pos2(600.0, 200.0))),
        ("a settings bar", Grab::Bar("Pitch range")),
    ] {
        for lose_it in [Lose::Pointer, Lose::Focus] {
            let moved = scroll_settings_after_lost_drag(grab, lose_it);
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

/// Where the doomed drag takes hold: a fixed point in a picture pane, or a
/// named settings bar found where it was painted — a bar draws its name at
/// its own left end, so a small offset from the text is inside the track.
#[derive(Clone, Copy, Debug)]
enum Grab {
    Point(egui::Pos2),
    Bar(&'static str),
}

/// Press and drag at `grab`, lose the release, then wheel over the settings
/// pane and answer how far its content moved.
fn scroll_settings_after_lost_drag(grab: Grab, lose: Lose) -> f32 {
    let mut state = fresh();
    unfold_the_readout_panes(&mut state);
    // The Analyzer settings, open in the Display tab.
    let tab = SettingsPane::Section(Section::Analyzer).install(&mut state);
    let path = state.workspace.dock.find_tab(&tab).expect("the Display tab");
    state.workspace.dock.set_active_tab(path).expect("selecting the tab");
    // Tall enough that the Analyzer's first bars are inside the settings leaf:
    // the section opens under the five headers stacked above it, and a bar
    // this fixture presses on outside the leaf is a press on the pane below.
    let screen_h = 640.0;
    let mut h = DockHarness::at(egui::vec2(1000.0, screen_h));
    let body =
        egui::Rect::from_min_max(egui::pos2(700.0, 20.0), egui::pos2(1000.0, screen_h * 0.55 + 2.0));
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
        before.iter().filter_map(|(text, pos)| after.get(text).map(|m| m.y - pos.y)).collect();
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
    let mut state = fresh();
    unfold_the_readout_panes(&mut state);
    // The Analyzer settings, open in the Display tab.
    let tab = SettingsPane::Section(Section::Analyzer).install(&mut state);
    let path = state.workspace.dock.find_tab(&tab).expect("the Display tab");
    state.workspace.dock.set_active_tab(path).expect("selecting the tab");
    // Tall enough that the Smoothing bar is on screen below the Display pane's
    // five section headers and the whole Plot section above it.
    let screen_h = 880.0;
    let mut h = DockHarness::at(egui::vec2(1000.0, screen_h));
    // The settings leaf, whose bars run the width of the column at x ~700..1000:
    // from under its tab bar down to the 0.55 split.
    let body =
        egui::Rect::from_min_max(egui::pos2(700.0, 20.0), egui::pos2(1000.0, screen_h * 0.55 + 2.0));
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
    let mut frame =
        |state: &mut SharedState, events: Vec<egui::Event>| h.frame(state, events);
    // Smoothing: a plain 0..=0.9 bar, so where the pointer is says what the
    // value should be, and the far end of the range is what an off-window drag
    // to the right must arrive at.
    let out = frame(&mut state, vec![]);
    let name =
        bar_named(&out, "Smoothing").expect("the Smoothing bar is drawn in the Analyzer section");
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
    let moved = wheel_over_settings_pane(SettingsPane::Tab(panes::Tab::Video), 600.0);
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
        SettingsPane::Tab(panes::Tab::Tuning),
        423.0,
        PROJECTIONS[0],
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
/// is a question about the wrapping: egui_dock puts each body in a `ScrollArea`
/// of its own, and a fixture that calls `TabViewer::ui` on a hand-built child
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
fn scrolling_settings_pane(pane: SettingsPane, scale: f32) -> Vec<egui::epaint::ClippedShape> {
    let mut state = fresh();
    let tab = pane.install(&mut state);
    state.workspace.dock = egui_dock::DockState::new(vec![tab]);
    // The same shell [`settings_pane_at_width`] draws for, so the Video pane
    // brings its record row and its progress bar — the two controls
    // `widgets::bar_width` calls out as having nowhere to wrap to, and so the
    // two likeliest to reach the lane.
    state.take.supported = true;
    state.take.last_ready = true;
    state.take.render_progress = Some(FIXTURE_RENDER);
    for i in 0..40 {
        state.console.log(format!("{i:02} a log line long enough to run the width of the pane"));
    }
    for note in 40..80 {
        state.tracker.handle_event(harmonigraph_core::NoteEvent::on(0.5, 0, note, 1.0));
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
    out.shapes
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
    assert!(bar <= margin + 0.01, "at {scale} a {bar}pt bar does not fit the {margin}pt gutter");
    let body = egui::Rect::from_min_max(
        egui::pos2(0.0, crate::theme::tab_bar_height(scale)),
        (SCROLLING_PANE * scale).to_pos2(),
    );
    for pane in SETTINGS_PANES {
        let shapes = scrolling_settings_pane(pane, scale);
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
        // scrolls, whose area IS the body. The two readout panes scroll in an
        // area of their own — a row that must not scroll sits above each — so
        // theirs stands one content margin further in.
        let own_area = matches!(
            pane,
            SettingsPane::Tab(panes::Tab::Console | panes::Tab::Notes)
        );
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
    state.workspace.dock = egui_dock::DockState::new(vec![panes::Tab::Tuning]);
    // Narrower than the two columns need, and tall enough that the pane does
    // not also scroll — one bar in the picture is one bar to find.
    let mut h = DockHarness::at(egui::vec2(120.0, 900.0));
    let screen = h.screen;
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
