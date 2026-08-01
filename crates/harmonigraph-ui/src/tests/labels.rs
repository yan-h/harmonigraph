//! Note labels: how a name and its comma marks stack over a node, and the
//! mark cache that hands their textures to egui.

use crate::*;
use harmonigraph_render::wgpu::TextureFormat;

/// The box one piece of text occupies.
fn text_box(texts: &[(egui::Rect, String)], want: &str) -> egui::Rect {
    texts
        .iter()
        .filter(|(_, t)| t == want)
        .map(|(r, _)| *r)
        .reduce(|a, b| a.union(b))
        .unwrap_or_else(|| panic!("no {want:?} drawn, got {texts:?}"))
}

/// A label's text pieces AND the boxes of its drawn marks.
///
/// The comma signs are geometry rather than type (see
/// [`panes::lattice::draw_stacked_name`]), so a text-only view of a label is
/// blind to exactly the marks these tests are about. Each drawn piece emits
/// two shapes, halo then fill, and both are symmetric about the piece, so a
/// box here is the piece's own box grown by the halo.
fn drawn_label(
    name: harmonigraph_core::NoteName,
    anchor: egui::Pos2,
) -> (Vec<(egui::Rect, String)>, Vec<egui::Rect>) {
    let (pieces, shapes) = label_pieces(name, anchor);
    (pieces.into_iter().map(|(galley, _, text)| (galley, text)).collect(), shapes)
}

/// The same label measured as INK — tight to the glyphs, with none of the
/// leading a galley box carries above and below them.
///
/// Which is the only measure that answers whether two rows of the column
/// actually clear each other: at these sizes the leading alone is worth more
/// than the air between the rows, so galley boxes overlap while the glyphs
/// sit comfortably apart.
fn drawn_label_ink(
    name: harmonigraph_core::NoteName,
    anchor: egui::Pos2,
) -> Vec<(egui::Rect, String)> {
    label_pieces(name, anchor).0.into_iter().map(|(_, ink, text)| (ink, text)).collect()
}

/// A label's ink, drawn at an arbitrary `scale` and pixels-per-point.
///
/// Both matter and only together: `draw_stacked_name` is called at 1 nowhere
/// in the app — the lattice pane scales by its own height, the camera and the
/// node's own size, and the spectral roll by `LABEL_PT / NAME_SIZE` of its
/// label setting, about 0.41 at the default. Down there a point of clearance
/// is a fraction of a device pixel, and what a glyph's quad rounds to decides
/// the last of it.
fn label_ink_at(
    name: harmonigraph_core::NoteName,
    anchor: egui::Pos2,
    scale: f32,
    ppp: f32,
) -> Vec<(egui::Rect, String)> {
    let ctx = egui::Context::default();
    theme::apply_theme(&ctx);
    ctx.set_pixels_per_point(ppp);
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
    let mut batch = crate::text::TextBatch::default();
    let _ = ctx.run_ui(
        egui::RawInput { screen_rect: Some(screen), ..Default::default() },
        |ui| {
            panes::lattice::draw_stacked_name(
                &mut batch,
                ui.painter(),
                anchor,
                name,
                egui::Color32::WHITE,
                egui::Color32::BLACK,
                scale,
                1.0,
            );
        },
    );
    batch.pieces().iter().map(|p| (p.ink, p.text.clone())).collect()
}

/// Every text piece of a label as (galley box, ink box, text), and the boxes
/// of its drawn marks.
fn label_pieces(
    name: harmonigraph_core::NoteName,
    anchor: egui::Pos2,
) -> (Vec<(egui::Rect, egui::Rect, String)>, Vec<egui::Rect>) {
    let ctx = egui::Context::default();
    theme::apply_theme(&ctx); // the real Iosevka metrics, not egui's fallback
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
    let mut batch = crate::text::TextBatch::default();
    let out = ctx.run_ui(
        egui::RawInput { screen_rect: Some(screen), ..Default::default() },
        |ui| {
            panes::lattice::draw_stacked_name(
                &mut batch,
                ui.painter(),
                anchor,
                name,
                egui::Color32::WHITE,
                egui::Color32::BLACK,
                1.0,
                1.0,
            );
        },
    );
    let texts = batch.pieces().iter().map(|p| (p.galley, p.ink, p.text.clone())).collect();
    let shapes = out
        .shapes
        .iter()
        .map(|s| s.shape.visual_bounding_rect())
        .filter(|r| r.is_finite() && r.width() > 0.0 && r.height() > 0.0)
        .collect();
    (texts, shapes)
}

/// The lattice's note labels stack the accidental over the comma sign in one
/// column after the letter, so a name deep in the lattice stays narrow. The
/// whole name still has to sit centered on its node.
#[test]
fn note_label_stacks_the_marks_and_stays_centered_on_the_node() {
    let anchor = egui::pos2(200.0, 200.0);
    let name = harmonigraph_core::NoteName {
        letter: 'C',
        sharps: 5,
        syntonic_commas: 4,
        septimal_commas: 0,
    };
    let (texts, shapes) = drawn_label(name, anchor);

    // Counted marks, not five sharps and four pluses spelled out. The `+`
    // itself is drawn, so only its COUNT is text -- and each count is its own
    // piece, so it can be set closer to its sign than an advance apart.
    let letter = text_box(&texts, "C");
    let accidental_sign = text_box(&texts, "\u{266F}");
    let accidental_count = text_box(&texts, "5");
    let accidental = accidental_sign.union(accidental_count);
    let count = text_box(&texts, "4");
    let sign = shapes
        .iter()
        .copied()
        .reduce(|a, b| a.union(b))
        .expect("the + should be drawn, not typeset");

    // One column, beginning where the letter ends. The boxes are ink, so
    // they meet within a glyph's own side bearing rather than within the
    // rim a stamped box would carry.
    const BEARING: f32 = 2.0;
    assert!(
        (accidental.left() - letter.right()).abs() <= 2.0 * BEARING,
        "marks should follow the letter ({accidental:?} after {letter:?})"
    );
    assert!(
        (accidental.left() - sign.left()).abs() <= 2.0 * BEARING,
        "the drawn sign shares the accidental's column ({sign:?} vs {accidental:?})"
    );
    // A count multiplies the sign beside it rather than continuing a word, so
    // it is tracked in by MARK_TRACK instead of taking a clear cell after it.
    // Both rows pin against the same number off the same cell, because both
    // set at MARK_SIZE and the typeset sign's box IS one cell -- which is
    // what the drawn sign under it claims too.
    let mark_size = panes::lattice::MARK_SIZE;
    let cell = panes::lattice::MARK_ADVANCE * mark_size;
    let track = panes::lattice::MARK_TRACK * mark_size;
    assert!(
        (accidental_count.left() - count.left()).abs() < 0.01,
        "the two counts should share a left edge ({accidental_count:?} vs {count:?})"
    );
    let cell_left = accidental_sign.center().x - cell / 2.0;
    assert!(
        (count.left() - (cell_left + cell - track)).abs() < 0.01,
        "a count should track {track} into its cell (count {count:?}, cell at {cell_left})"
    );
    // ...but never so far that it climbs onto the sign.
    //
    // Bounded against the drawn sign's INK -- MARK_INK_W wide, centered in
    // its cell -- on both counts. `sign` here is the shape box, which carries
    // a halo wide enough to swallow the overlap this is looking for; and a
    // tolerance that includes `track` cannot be failed by tracking at all,
    // which is what this assertion did before: MARK_TRACK could be taken to
    // 0.35, sitting the digit squarely on the `+` bar, with every label test
    // still green.
    let sign_ink_right = cell_left + cell / 2.0 + panes::lattice::MARK_INK_W * mark_size / 2.0;
    let count_ink = text_box(&drawn_label_ink(name, anchor), "4");
    assert!(
        count_ink.left() >= sign_ink_right,
        "the count should not climb onto its sign (count ink {count_ink:?}, \
         sign ink ends at {sign_ink_right})"
    );
    // Superscript over subscript, straddling the letter's own line.
    assert!(accidental.center().y < letter.center().y, "the accidental rides high");
    assert!(sign.center().y > letter.center().y, "the comma sits low");
    // Marks are subordinate to the letter, not the same weight...
    assert!(accidental.height() < letter.height(), "marks are the smaller size");
    // ...and neither stands proud of it: the stacked pair has to stay inside
    // the letter's own height, or the label reads as two lines, not one name.
    assert!(
        accidental.top() >= letter.top() - 0.01 && count.bottom() <= letter.bottom() + 0.01,
        "marks should not overhang the letter (acc {accidental:?}, count {count:?}, \
         letter {letter:?})"
    );

    // The name as a whole straddles the node it labels.
    let name_box = letter.union(accidental).union(count);
    assert!(
        (name_box.center().x - anchor.x).abs() < 0.5,
        "name should center on the node ({name_box:?} vs {anchor:?})"
    );
    // ...and stays about as wide as two letters, which is the whole point of
    // counting the marks rather than repeating them.
    assert!(
        name_box.width() < letter.width() * 2.5,
        "a deep name should still fit a node, got {}",
        name_box.width()
    );
}

/// The two rows of the mark column set at ONE size, and clear each other as
/// INK.
///
/// Both halves matter and they pull against each other: the air between the
/// rows is bought with `MARK_RISE` alone, because the other
/// way to buy it — setting the accidental smaller, since `♯` carries the
/// tallest ink in the column — makes the row visibly the smaller of the two.
/// The count digits are where that shows: one directly over the other, same
/// column, nothing between them to excuse a size difference.
///
/// Counted on both rows is also the tightest the column ever gets, since a
/// count digit reaches further down, and further up, than the `+` beside it.
#[test]
fn the_two_mark_rows_set_alike_and_clear_each_other() {
    let anchor = egui::pos2(200.0, 200.0);
    let name = harmonigraph_core::NoteName {
        letter: 'C',
        sharps: 3,
        syntonic_commas: 2,
        septimal_commas: 0,
    };
    let ink = drawn_label_ink(name, anchor);
    let sharp = text_box(&ink, "\u{266F}");
    let sharp_count = text_box(&ink, "3");
    let comma_count = text_box(&ink, "2");

    // Two digits, one per row: the pair that gives a size mismatch away.
    assert!(
        (sharp_count.height() - comma_count.height()).abs() < 0.01,
        "the two rows should set at one size ({sharp_count:?} vs {comma_count:?})"
    );
    // Same column, and the tightest pair in the label.
    assert!(
        sharp_count.bottom() < comma_count.top(),
        "a count should clear the count below it ({sharp_count:?} over {comma_count:?})"
    );
    // The `♯` sits a column to the left of that digit, so nothing collides
    // either way -- but the two ROWS should not interleave at all, or the
    // stack stops reading as one row over another.
    assert!(
        sharp.bottom() < comma_count.top(),
        "the accidental should sit entirely above the comma row \
         ({sharp:?} over {comma_count:?})"
    );

    // And it has to hold at the scales the label is actually DRAWN at, on a
    // Retina pixel grid as much as a plain one -- a rise tuned at 1 says
    // nothing about 0.4, where the roll draws.
    //
    // The floor is 0.35 because below it quantization decides the outcome
    // rather than the rise does: a glyph's quad rounds to whole device
    // pixels, so the clearance stops shrinking smoothly and starts jittering
    // by half a point either way. Sampling every 0.005 down to 0.08, the
    // quads of these two digits touch at a handful of scales under 0.24 --
    // and they do so at MARK_RISE = 1.0, the loosest the rows can sit, as
    // well. That is the rasterizer's floor, not this constant's, and no
    // value here buys past it.
    for ppp in [1.0, 2.0] {
        for step in 0..=25 {
            let scale = 0.35 + step as f32 * 0.05;
            let ink = label_ink_at(name, anchor, scale, ppp);
            let over = text_box(&ink, "3");
            let under = text_box(&ink, "2");
            assert!(
                over.bottom() < under.top(),
                "counts collide at scale {scale} on a {ppp}x grid \
                 ({over:?} over {under:?})"
            );
        }
    }
}

/// The septimal mark sits ACROSS the divide between the accidental and the
/// comma, not in either slot.
///
/// It belongs to a different prime than the two it sits beside, and it is
/// placed to say so: centered on the letter's own line, with air before it.
/// It used to take one slot or the other as a second cue for its direction,
/// which put it in the stack it is not part of; the chevron carries its own
/// direction, so the slot is free to mean this instead.
#[test]
fn the_septimal_mark_sits_across_the_divide_between_the_other_two() {
    let anchor = egui::pos2(200.0, 200.0);
    let mark_of = |septimal_commas: i32| {
        let name = harmonigraph_core::NoteName {
            letter: 'B',
            sharps: -1,
            syntonic_commas: 0,
            septimal_commas,
        };
        let (_, shapes) = drawn_label(name, anchor);
        shapes.into_iter().reduce(|a, b| a.union(b)).expect("a septimal mark should be drawn")
    };
    // Both directions sit on the same line, and it is the letter's own.
    for commas in [-1, 1] {
        let mark = mark_of(commas);
        assert!(
            (mark.center().y - anchor.y).abs() < 1.0,
            "a septimal mark belongs on the letter's line, got {mark:?} against {anchor:?}"
        );
    }
    // A home-sheet name draws no mark at all.
    let (_, none) = drawn_label(
        harmonigraph_core::NoteName {
            letter: 'B',
            sharps: -1,
            syntonic_commas: 0,
            septimal_commas: 0,
        },
        anchor,
    );
    assert!(none.is_empty(), "no sevens component, no mark: {none:?}");
}

/// A mark costs a bounded few quads, exactly as the glyphs beside it do.
///
/// This is the invariant `crate::text` exists to hold: a label's rim is
/// stamped in the FRAGMENT stage precisely because "20 copies of every glyph
/// was most of the geometry in a busy frame". `paint_mark` reasoned its way
/// out of that with "a mark is one quad, so the loop is affordable here" --
/// true of the handful of hovered and sounding nodes it was written against,
/// and false the moment note names put a mark on every roll ribbon and every
/// lit node of a collapsed 12-EDO lattice.
///
/// A count, not a timing, so it cannot go quiet on a fast machine.
#[test]
fn a_mark_costs_a_bounded_number_of_quads() {
    let anchor = egui::pos2(200.0, 200.0);
    let name = harmonigraph_core::NoteName {
        letter: 'E',
        sharps: 0,
        syntonic_commas: -1,
        septimal_commas: -1,
    };
    let (_, shapes) = drawn_label(name, anchor);
    assert!(
        shapes.len() <= 4,
        "two marks cost {} quads; the rim belongs in the fragment stage, \
         not once per stamp in the shape list",
        shapes.len()
    );
}

/// The septimal mark gets a column of its own, so a name carrying both
/// commas reads as three pieces rather than a pile.
#[test]
fn both_comma_marks_get_their_own_column() {
    let anchor = egui::pos2(200.0, 200.0);
    let name = harmonigraph_core::NoteName {
        letter: 'E',
        sharps: 0,
        syntonic_commas: -1,
        septimal_commas: -1,
    };
    let (texts, shapes) = drawn_label(name, anchor);
    assert!(texts.iter().all(|(_, t)| t == "E"), "single marks carry no count: {texts:?}");
    let letter = text_box(&texts, "E");
    // Two drawn marks, so two columns' worth of shapes: the syntonic bar
    // sits left of the septimal shape rather than on top of it.
    let left = shapes.iter().copied().reduce(|a, b| a.union(b)).expect("marks drawn");
    assert!(left.left() >= letter.right() - 2.0, "marks follow the letter, {left:?}");

    // Cluster the stamps into columns rather than reading the flat list's
    // extremes. A mark is not one shape: `paint_mark` stamps its rim as ~20
    // separate quads around the fill, so ONE mark already spreads its centers
    // over four points, and `min(x) < max(x)` holds with the other mark
    // missing entirely or drawn on top of it -- the two failures this test
    // exists to catch. Within a mark consecutive stamps are under a point
    // apart; between the columns they are nearly two.
    const COLUMN_GAP: f32 = 1.0;
    let mut stamps: Vec<egui::Pos2> = shapes.iter().map(|r| r.center()).collect();
    stamps.sort_by(|a, b| a.x.total_cmp(&b.x));
    let mut columns: Vec<Vec<egui::Pos2>> = Vec::new();
    for stamp in stamps {
        match columns.last_mut() {
            Some(col) if stamp.x - col[col.len() - 1].x <= COLUMN_GAP => col.push(stamp),
            _ => columns.push(vec![stamp]),
        }
    }
    let widths: Vec<usize> = columns.iter().map(|c| c.len()).collect();
    assert_eq!(columns.len(), 2, "two marks, two columns, got {widths:?}");
    assert_eq!(widths[0], widths[1], "each column is a whole mark, not one mark's rim");

    // The right-hand column is the septimal one, and it is on the letter's own
    // line while the syntonic bar sits below it -- so which column is which is
    // checked, not assumed from the ordering the split already imposed.
    let line = |col: &[egui::Pos2]| col.iter().map(|p| p.y).sum::<f32>() / col.len() as f32;
    assert!(
        line(&columns[1]) < line(&columns[0]),
        "the septimal mark takes the right column, across the letter's line: {:?}",
        columns.iter().map(|c| line(c)).collect::<Vec<_>>()
    );
}

/// A plain name has no marks to stack -- nothing extra is drawn, and the
/// letter alone centers on the node.
#[test]
fn a_natural_note_label_is_just_the_letter() {
    let anchor = egui::pos2(200.0, 200.0);
    let name = harmonigraph_core::NoteName {
        letter: 'G',
        sharps: 0,
        syntonic_commas: 0,
        septimal_commas: 0,
    };
    let (texts, shapes) = drawn_label(name, anchor);
    assert!(texts.iter().all(|(_, t)| t == "G"), "only the letter: {texts:?}");
    assert!(shapes.is_empty(), "a natural draws no marks: {shapes:?}");
    assert!((text_box(&texts, "G").center().x - anchor.x).abs() < 0.5);
}

/// Every label the lattice draws, at one camera and one Size bar setting: the
/// rasterized type size, and the ink it actually covers.
fn lattice_labels_at(label_scale: f32, distance: f32, ppp: f32) -> Vec<(f32, egui::Rect)> {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.view.show_labels = true;
    state.view.label_scale = label_scale;
    state.camera.distance = distance;
    // Middle C: the origin node, which the camera looks straight at.
    state.tracker.handle_event(harmonigraph_core::NoteEvent {
        time: 0.0,
        channel: 0,
        note: 60,
        kind: harmonigraph_core::NoteEventKind::On { velocity: 1.0 },
    });
    let scene = harmonigraph_scene::derive_scene(
        &state.tracker,
        &state.tuning,
        &state.view,
        &state.frame_params,
        state.camera,
        None,
        0.0,
    );

    let ctx = egui::Context::default();
    theme::apply_theme(&ctx); // the real Iosevka metrics, not egui's fallback
    ctx.set_pixels_per_point(ppp);
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 800.0));
    let mut batch = crate::text::TextBatch::default();
    let _ = ctx.run_ui(
        egui::RawInput { screen_rect: Some(rect), time: Some(0.0), ..Default::default() },
        |ui| panes::lattice::draw_node_labels(ui, rect, &scene, &state.view, &mut batch),
    );
    batch.pieces().iter().map(|p| (p.font_size, p.ink)).collect()
}

/// Past the raster ceiling, asking for bigger type draws no bigger.
///
/// [`crate::text::MAX_GLYPH_PX`] bounds what a label is RASTERIZED at, and
/// `text::ladder` exists so the magnification is bounded with it: it clamps the
/// request before dividing, so past the ceiling the factor is exactly 1 and the
/// drawn size stops where the raster does. A caller that snaps and then divides
/// the RAW request instead absorbs everything past the ceiling into the
/// magnification, which draws a bitmap the sampler has to stretch — not big
/// type, blurred type. `ladder`'s own doc names this as the reason it is a
/// function rather than two lines at each call site.
///
/// Both settings here are past the ceiling, so both must rasterize at it and
/// draw the same size. `want` is `pane/860 · label_scale · screen_scale`, which
/// at an 800pt pane and the camera at `MIN_DISTANCE` (`screen_scale` 6) is
/// `5.58 · label_scale` against a ceiling of `512/(30·ppp)` = 8.53 at `ppp` 2.
///
/// Retina is what makes this ordinary rather than a corner: the ceiling is
/// crossed at half the zoom on a 2x display, so the same camera and the same
/// Size bar are sharp on an external monitor and soft on the laptop panel —
/// and dragging the plugin window between the two crosses it with nothing
/// touched.
#[test]
fn a_zoom_past_the_raster_ceiling_stops_growing_the_drawn_label() {
    let distance = harmonigraph_scene::Camera::MIN_DISTANCE;
    let just_past = lattice_labels_at(1.6, distance, 2.0);
    let far_past = lattice_labels_at(3.0, distance, 2.0);
    assert!(!just_past.is_empty() && !far_past.is_empty(), "the held C should be labeled");

    // The raster is clamped either way -- this is what puts both settings past
    // the ceiling rather than merely at different zooms.
    let biggest = |v: &[(f32, egui::Rect)]| v.iter().map(|(s, _)| *s).fold(0.0, f32::max);
    assert_eq!(
        biggest(&just_past),
        biggest(&far_past),
        "both settings must rasterize at the ceiling for this to measure the magnification",
    );

    // So the ink must match too. It is the magnification that differs, and it
    // is the magnification that should have been clamped alongside the raster.
    let tallest = |v: &[(f32, egui::Rect)]| {
        v.iter().map(|(_, ink)| ink.height()).fold(0.0, f32::max)
    };
    let (a, b) = (tallest(&just_past), tallest(&far_past));
    assert!(
        (a - b).abs() <= a * 0.01,
        "the Size bar kept growing the drawn label past the raster ceiling: \
         {a} then {b}, a factor of {:.2} stretched out of one bitmap",
        b / a,
    );
}

/// The cents readout hangs off the note name's GLYPHS, not its galley box --
/// a monospace line box carries several pixels of leading below the letter,
/// and spacing box-to-box left the readout visibly adrift from the name it
/// belongs to.
#[test]
fn the_cents_readout_sits_right_under_the_note_name() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.view.show_labels = true;
    state.view.show_cents = true;
    // Middle C: the origin node, which the default camera looks straight at.
    state.tracker.handle_event(harmonigraph_core::NoteEvent {
        time: 0.0,
        channel: 0,
        note: 60,
        kind: harmonigraph_core::NoteEventKind::On { velocity: 1.0 },
    });
    let scene = harmonigraph_scene::derive_scene(
        &state.tracker,
        &state.tuning,
        &state.view,
        &state.frame_params,
        state.camera,
        None,
        0.0,
    );

    let ctx = egui::Context::default();
    theme::apply_theme(&ctx); // the real Iosevka metrics, not egui's fallback
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 800.0));
    let mut batch = crate::text::TextBatch::default();
    let _ = ctx.run_ui(
        egui::RawInput { screen_rect: Some(rect), time: Some(0.0), ..Default::default() },
        |ui| panes::lattice::draw_node_labels(ui, rect, &scene, &state.view, &mut batch),
    );

    // A held note lights every node of its pitch class, so each piece turns
    // up once per lit node. Sort them by the label's own type sizes, which
    // nothing else in the pane shares -- not by the text, since one pitch
    // class is spelled several ways across the lattice.
    let ink_of = |want: &[f32]| -> Vec<egui::Rect> {
        let mut clusters: Vec<egui::Rect> = Vec::new();
        for piece in batch.pieces().iter().filter(|p| want.contains(&p.font_size)) {
            match clusters.iter_mut().find(|seen| seen.intersects(piece.ink)) {
                Some(seen) => *seen = seen.union(piece.ink),
                None => clusters.push(piece.ink),
            }
        }
        clusters
    };
    // Every size here scales with the pane and the camera together, so read
    // that scale off the biggest piece drawn rather than assuming the pane is
    // the one the constants are quoted at.
    let scale = batch.pieces().iter().map(|p| p.font_size).fold(0.0, f32::max)
        / panes::lattice::NAME_SIZE;
    // Letter and marks together: the readout has to clear the comma, which
    // hangs lower than the letter does.
    let names = ink_of(&[panes::lattice::NAME_SIZE * scale, panes::lattice::MARK_SIZE * scale]);
    let cents = ink_of(&[panes::lattice::CENTS_SIZE * scale]);
    assert!(!names.is_empty() && !cents.is_empty(), "the held C should be labeled");

    // Every readout belongs to the name directly above it, and sits the
    // intended air below it -- not the wider, font-dependent gap that
    // box-to-box spacing left behind (6px against a 1px constant).
    for readout in &cents {
        let name = names
            .iter()
            // Overlap, not equal centers: on a node whose name carries marks
            // the letter is pushed left to make room for the mark column,
            // while the readout stays centered on the node itself.
            .filter(|n| n.left() < readout.right() && n.right() > readout.left())
            .filter(|n| n.bottom() <= readout.top())
            .min_by(|a, b| (readout.top() - a.bottom()).total_cmp(&(readout.top() - b.bottom())))
            .unwrap_or_else(|| panic!("no name above {readout:?}, of {names:?}"));
        let gap = readout.top() - name.bottom();
        // Slack enough for a DRAWN comma hanging below the letter's ink: the
        // readout clears it (`draw_stacked_name` reports it) but the clusters
        // above cannot see it, a drawn mark being a bitmap rather than a
        // glyph. It is a fraction of the type, so the slack is quoted against
        // the gap rather than in absolute points — the regression this is
        // watching for, hanging the readout off the galley box instead of the
        // ink, is worth twice the gap and clears any of this.
        let want = panes::lattice::CENTS_GAP * scale;
        let slack = want / 3.0;
        assert!(
            (gap - want).abs() <= slack,
            "cents should sit CENTS_GAP under the name, got {gap}px of ink-to-ink gap"
        );
    }
}

/// The lattice's label-size bar and the clamp its value is persisted through
/// offer the same range.
///
/// `SCALE_BAR_RANGE` says it is "one range for the three of them", and for two
/// of them it is the constant itself: `sane_scale` fits `marking_scale` and
/// `note_name_scale` to it on load, so those two cannot drift and are not
/// worth asserting — an assertion that clamping to a constant lands inside it
/// only restates `clamp`. The lattice's `label_scale` is the one that can:
/// `ViewConfig` lives in `harmonigraph-scene`, which is BELOW this crate, so the
/// range is not visible there and `migrate_legacy` clamps to a written-out
/// copy of the same two numbers.
///
/// Nothing ties the copy to the original. Widen the bar and a saved view keeps
/// loading at the old ceiling, which is a setting that will not stay where it
/// is put — silently, and only for the one of the three that is a different
/// crate. This is what notices.
#[test]
fn the_lattice_label_bar_persists_through_the_range_it_offers() {
    let through_view = |scale: f32| {
        let mut view = harmonigraph_scene::ViewConfig { label_scale: scale, ..Default::default() };
        view.sanitize();
        view.label_scale
    };
    let (low, high) = (*SCALE_BAR_RANGE.start(), *SCALE_BAR_RANGE.end());
    assert_eq!(through_view(low - 1.0), low, "the bar's floor");
    assert_eq!(through_view(high + 1.0), high, "...and its ceiling");
}

/// Draw enough distinct marks in ONE pass to outrun the cache limit, so
/// eviction is running while the pass is still drawing. Returns the texture
/// ids the pass drew, and the ids it asked egui to destroy.
fn mark_cache_pass(ctx: &egui::Context) -> (std::collections::HashSet<egui::TextureId>, Vec<egui::TextureId>) {
    use panes::lattice::{MARK_CACHE_LIMIT, MARK_SIZE};
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
    // Both comma marks, so each size contributes more than one key.
    let name =
        harmonigraph_core::NoteName { letter: 'C', sharps: 1, syntonic_commas: 1, septimal_commas: 1 };
    let out = ctx.run_ui(
        egui::RawInput { screen_rect: Some(screen), ..Default::default() },
        |ui| {
            let mut batch = crate::text::TextBatch::default();
            // One mark per whole pixel size, far enough past the limit that
            // the pass evicts marks it has already painted.
            for size_px in 3..=(3 + MARK_CACHE_LIMIT / 2 + 8) {
                panes::lattice::draw_stacked_name(
                    &mut batch,
                    ui.painter(),
                    egui::pos2(200.0, 200.0),
                    name,
                    egui::Color32::WHITE,
                    egui::Color32::BLACK,
                    size_px as f32 / MARK_SIZE,
                    1.0,
                );
            }
        },
    );
    let drawn = out
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Mesh(mesh) => Some(mesh.texture_id),
            _ => None,
        })
        .collect();
    (drawn, out.textures_delta.free)
}

/// A pass that fills the mark cache must not destroy a texture it has
/// already drawn.
///
/// Eviction drops the last handle to a bitmap, which makes egui queue that
/// id into `textures_delta.free` -- and egui-baseview's wgpu renderer
/// applies those frees BEFORE it submits the encoder. So a mark evicted
/// midway through a pass is destroyed while the draw commands naming it are
/// still queued, and `Queue::submit` fails validation with "Texture ... has
/// been destroyed", which wgpu treats as fatal. The victim is arbitrary, so
/// it is sometimes a mark this pass has already painted.
///
/// Reachable from any control that walks a mark's key: the sizes zooming
/// steps through, or the weight when that was still a setting. Each pass
/// mints fresh keys, so the cache sits at its limit and evicts on every
/// insert.
#[test]
fn filling_the_mark_cache_never_frees_a_texture_the_pass_drew() {
    let ctx = egui::Context::default();
    theme::apply_theme(&ctx);
    let (drawn, freed) = mark_cache_pass(&ctx);
    let bad: Vec<_> = freed.iter().copied().filter(|id| drawn.contains(id)).collect();
    assert!(bad.is_empty(), "the pass freed {} textures it had drawn: {bad:?}", bad.len());
}

/// ...and the pass AFTER it must actually destroy them.
///
/// The retention is a delay, not a reprieve: holding an evicted bitmap
/// forever would trade the crash above for a leak that grows for as long as
/// the editor is open, since a zoom drag mints fresh keys every pass. This
/// pins the half the single-pass test cannot see -- with the prune deleted
/// that test still passes, because pass 0 frees nothing either way.
#[test]
fn the_next_pass_destroys_what_the_last_one_retired() {
    let ctx = egui::Context::default();
    theme::apply_theme(&ctx);
    let (_, freed_first) = mark_cache_pass(&ctx);
    assert!(freed_first.is_empty(), "the first pass should hold its evictions, freed {freed_first:?}");

    let (drawn, freed_second) = mark_cache_pass(&ctx);
    assert!(
        !freed_second.is_empty(),
        "the second pass should destroy what the first retired, or they accumulate"
    );
    let bad: Vec<_> = freed_second.iter().copied().filter(|id| drawn.contains(id)).collect();
    assert!(bad.is_empty(), "the second pass freed {} textures it had drawn: {bad:?}", bad.len());
}
