//! Note labels: how a name and its comma marks stack over a node.

use super::probe::{self, fresh};
use crate::*;

/// The window a stacked name is drawn on. Room for a label at the anchors
/// below and no more: nothing here reads the window, only the ink.
const NAME_SCREEN: egui::Vec2 = egui::vec2(400.0, 400.0);

/// The pane the lattice's own labels are drawn into.
const PANE: egui::Rect = egui::Rect { min: egui::pos2(0.0, 0.0), max: egui::pos2(1000.0, 800.0) };

/// One stacked name drawn into a [`crate::text::TextBatch`], and what
/// [`marks::draw_stacked_name`] reported as its reach below the anchor.
///
/// The BATCH is what every reading in this file is off, rather than the shape
/// list: a mark is geometry cut from a sheet of its own, so a name's letter,
/// its counts and its signs only meet in there.
///
/// `scale` is what the name is asked for, `magnify` what it is drawn at
/// against the size it was rasterized at, and `ppp` the panel's device pixels
/// per point — see the callers, each of which drives exactly one of the three.
fn stacked_name(
    name: harmonigraph_core::NoteName,
    anchor: egui::Pos2,
    scale: f32,
    magnify: f32,
    ppp: f32,
) -> (crate::text::TextBatch, f32, egui::FullOutput) {
    let mut batch = crate::text::TextBatch::default();
    let mut reach = 0.0;
    let out = probe::frame_full(&probe::themed_at(ppp), NAME_SCREEN, |ui| {
        reach = marks::draw_stacked_name(
            &mut batch,
            ui.painter(),
            anchor,
            name,
            egui::Color32::WHITE,
            egui::Color32::BLACK,
            scale,
            magnify,
            marks::NameLead::Centred,
        );
    });
    (batch, reach, out)
}

/// Every label the lattice pane draws for `scene`, in one batch, at `ppp`.
fn pane_labels(
    scene: &harmonigraph_scene::Scene,
    view: &harmonigraph_scene::ViewConfig,
    ppp: f32,
) -> crate::text::TextBatch {
    let mut batch = crate::text::TextBatch::default();
    let _ = probe::frame_full(&probe::themed_at(ppp), PANE.size(), |ui| {
        panes::lattice::draw_node_labels(ui, PANE, scene, view, &mut batch);
    });
    batch
}

/// The lattice's labels as the render pass is handed them: a batch built by
/// `draw_node_labels` into `rect` and turned into [`LatticeLabels`] inside the
/// SAME frame, which is the only place a painter exists to measure the glyphs
/// with.
///
/// `rect` is a parameter because where the pane sits in the window is one of
/// the questions: the pass is handed positions relative to the pane's own
/// corner, so the same picture in two places has to come out the same.
///
/// [`LatticeLabels`]: harmonigraph_render::LatticeLabels
fn lattice_labels_in(
    rect: egui::Rect,
    scene: &harmonigraph_scene::Scene,
    state: &SharedState,
) -> harmonigraph_render::LatticeLabels {
    let mut batch = crate::text::TextBatch::default();
    let mut labels = None;
    let _ = probe::painted_full(PANE.size(), |ui| {
        panes::lattice::draw_node_labels(ui, rect, scene, &state.view, &mut batch);
        labels = Some(batch.lattice_labels(ui.painter(), rect, state));
    });
    labels.expect("the closure runs")
}

/// The box one piece of text occupies.
fn text_box(texts: &[(egui::Rect, String)], want: &str) -> egui::Rect {
    texts
        .iter()
        .filter(|(_, t)| t == want)
        .map(|(r, _)| *r)
        .reduce(|a, b| a.union(b))
        .unwrap_or_else(|| panic!("no {want:?} drawn, got {texts:?}"))
}

/// A label's text pieces AND the quads of its drawn marks.
///
/// EVERY sign is geometry rather than type -- accidental and comma alike (see
/// [`marks::draw_stacked_name`]) -- so a text-only view of a label
/// sees only the letter and the counts, and is blind to exactly the marks
/// these tests are about. A mark is one quad, cut from a sheet of its own and
/// haloed by the same shader that haloes the letters, so there is nothing to
/// pair up here: the list is one entry per mark, in the order the label drew
/// them.
fn drawn_label(
    name: harmonigraph_core::NoteName,
    anchor: egui::Pos2,
) -> (Vec<(egui::Rect, String)>, Vec<egui::Rect>) {
    let (pieces, marks, _) = label_pieces(name, anchor, 1.0, 1.0);
    (pieces.into_iter().map(|(galley, _, text)| (galley, text)).collect(), marks)
}

/// One box per drawn mark: its own INK, without the clear margin its bitmap
/// carries.
///
/// A mark's QUAD is a pixel wider than the mark on every side
/// ([`MARK_BITMAP_PAD`](marks::MARK_BITMAP_PAD), which is there so a
/// sliding mark's edge fades instead of snapping), and every question asked
/// here -- which column a mark sits in, whether two rows clear each other --
/// is about where the mark IS. Shrinking once here is what keeps those
/// readings about the mark rather than about its margin. A point per side
/// exactly, because every fixture in this file draws at ppp 1 and magnify 1.
fn mark_fills(marks: &[egui::Rect]) -> Vec<egui::Rect> {
    let pad = marks::MARK_BITMAP_PAD as f32;
    marks.iter().map(|mark| mark.shrink(pad)).collect()
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
    label_pieces(name, anchor, 1.0, 1.0).0.into_iter().map(|(_, ink, text)| (ink, text)).collect()
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
    let (batch, ..) = stacked_name(name, anchor, scale, 1.0, ppp);
    batch.pieces().iter().map(|p| (p.ink, p.text.clone())).collect()
}

/// Every text piece of a label as (galley box, ink box, text), and the quads
/// of its drawn marks.
///
/// `magnify` is what the label is DRAWN at against the size it was rasterized
/// at — the split [`crate::text::TextBatch::magnified`] exists for. It now
/// reaches the text pieces and the drawn marks by ONE route, the batch's own
/// transform, and a fixture that leaves it at 1 sees nothing of it either way.
///
/// `ppp` is the panel's device pixels per point, which the marks cross twice:
/// they are rasterized in pixels and drawn in points, so the batch divides the
/// bitmap's size back out by it. At 1 that division is the identity and a
/// fixture cannot see it at all.
///
/// The third return is what `draw_stacked_name` reports as the label's reach
/// below the anchor, which is the caller's whole view of the drawn marks: the
/// cents readout is placed off it and nothing else asks.
fn label_pieces(
    name: harmonigraph_core::NoteName,
    anchor: egui::Pos2,
    magnify: f32,
    ppp: f32,
) -> (Vec<(egui::Rect, egui::Rect, String)>, Vec<egui::Rect>, f32) {
    let (batch, reach, _) = stacked_name(name, anchor, 1.0, magnify, ppp);
    let texts = batch.pieces().iter().map(|p| (p.galley, p.ink, p.text.clone())).collect();
    (texts, batch.marks().to_vec(), reach)
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

    // Counted marks, not five sharps and four pluses spelled out. Both signs
    // are drawn, so only their COUNTS are text -- and each count is its own
    // piece, so it can be set closer to its sign than an advance apart.
    let letter = text_box(&texts, "C");
    let accidental_count = text_box(&texts, "5");
    let count = text_box(&texts, "4");
    let fills = mark_fills(&shapes);
    let [accidental_sign, sign] = fills[..] else {
        panic!("both signs should be drawn, not typeset: {shapes:?}")
    };
    let accidental = accidental_sign.union(accidental_count);

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
    // set at MARK_SIZE and each drawn sign is centered in one cell.
    let mark_size = marks::MARK_SIZE;
    let cell = marks::MARK_ADVANCE * mark_size;
    let track = marks::MARK_TRACK * mark_size;
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
    // its cell -- on both counts. A tolerance that includes `track` cannot be
    // failed by tracking at all, which is what this assertion did before:
    // MARK_TRACK could be taken to 0.35, sitting the digit squarely on the `+`
    // bar, with every label test still green.
    //
    // MARK_INK_W is the `♯`'s width as much as the `+`'s -- the face gives
    // both 372/1000 em -- so one bound covers the pair. The `♭` is narrower
    // and only ever clears by more.
    let sign_ink_right = cell_left + cell / 2.0 + marks::MARK_INK_W * mark_size / 2.0;
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
    let (pieces, shapes, _) = label_pieces(name, anchor, 1.0, 1.0);
    let ink: Vec<(egui::Rect, String)> =
        pieces.into_iter().map(|(_, ink, text)| (ink, text)).collect();
    // The `♯`'s own bitmap, which is the whole of the mark: a drawn sign has
    // no galley, and the bitmap is a tighter box than one anyway.
    let sharp = mark_fills(&shapes)[0];
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
        // The `♭` is drawn too, so the septimal mark is the SECOND of the two
        // -- named by its place in the draw order rather than taken as the
        // only thing in the list.
        let fills = mark_fills(&shapes);
        let [_accidental, septimal] = fills[..] else {
            panic!("the flat and the septimal mark should both be drawn: {shapes:?}")
        };
        septimal
    };
    // Both directions sit on the same line, and it is the letter's own.
    for commas in [-1, 1] {
        let mark = mark_of(commas);
        assert!(
            (mark.center().y - anchor.y).abs() < 1.0,
            "a septimal mark belongs on the letter's line, got {mark:?} against {anchor:?}"
        );
    }
    // A home-sheet name draws no septimal mark -- the same `B♭`, one column
    // shorter. Counted rather than checked for emptiness, since the accidental
    // is itself a drawn mark and the list is never bare.
    let (_, home) = drawn_label(
        harmonigraph_core::NoteName {
            letter: 'B',
            sharps: -1,
            syntonic_commas: 0,
            septimal_commas: 0,
        },
        anchor,
    );
    assert_eq!(
        mark_fills(&home).len(),
        1,
        "no sevens component, no septimal mark -- just the flat: {home:?}"
    );
}

/// A mark costs ONE quad, exactly as a glyph beside it does — and it puts
/// nothing at all on the painter.
///
/// Two claims in one count. The first is the invariant `crate::text` exists to
/// hold: a label's rim is stamped in the FRAGMENT stage precisely because "20
/// copies of every glyph was most of the geometry in a busy frame", and a mark
/// that took its rim as geometry, or as a second bitmap under a second quad,
/// would be paying for it once per roll ribbon and once per lit node of a
/// collapsed 12-EDO lattice.
///
/// The second is #207's own: what a label emits onto the PAINTER is what a
/// nearer node cannot cover, since the painter draws over the finished
/// picture. Zero is the only answer that leaves the marks coverable.
///
/// A count, not a timing, so it cannot go quiet on a fast machine.
#[test]
fn a_mark_is_one_quad_in_the_batch_and_nothing_on_the_painter() {
    let anchor = egui::pos2(200.0, 200.0);
    let name = harmonigraph_core::NoteName {
        letter: 'E',
        sharps: 0,
        syntonic_commas: -1,
        septimal_commas: -1,
    };
    let (batch, _, out) = stacked_name(name, anchor, 1.0, 1.0, 1.0);
    assert_eq!(batch.marks().len(), 2, "two marks: {:?}", batch.marks());
    // The letter, the two signs, and no count digit — both commas are single.
    assert_eq!(batch.len(), 3, "a mark is one instance, like the letter beside it");
    let painted: Vec<_> = out
        .shapes
        .iter()
        .map(|s| s.shape.visual_bounding_rect())
        .filter(|r| r.is_finite() && r.width() > 0.0 && r.height() > 0.0)
        .collect();
    assert!(painted.is_empty(), "a label draws nothing on the painter, got {painted:?}");
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
    let (texts, marks) = drawn_label(name, anchor);
    assert!(texts.iter().all(|(_, t)| t == "E"), "single marks carry no count: {texts:?}");
    let letter = text_box(&texts, "E");
    // Two drawn marks, so two columns: the syntonic bar sits left of the
    // septimal shape rather than on top of it.
    //
    // Off the marks' own ink, not their quads: a mark's quad carries the clear
    // margin its bitmap is padded by, and the halo the shader paints reaches
    // further still and is SUPPOSED to run back over the letter, the way a
    // glyph's does. Neither says which column the mark is in.
    let fills = mark_fills(&marks);
    let [syntonic, septimal] = fills[..] else { panic!("both commas should be drawn: {marks:?}") };
    let left = syntonic.union(septimal);
    assert!(left.left() >= letter.right() - 2.0, "marks follow the letter, {left:?}");

    // Clear of each other left to right, rather than the one column that a
    // mark drawn on top of its neighbour (or missing entirely) would leave.
    assert!(
        syntonic.right() <= septimal.left(),
        "two marks, two columns: {syntonic:?} then {septimal:?}",
    );
    // And which is which is checked, not assumed from the order they were
    // drawn in: the septimal mark is the one on the letter's own line, while
    // the syntonic bar sits below it.
    assert!(
        septimal.center().y < syntonic.center().y,
        "the septimal mark takes the right column, across the letter's line: \
         {septimal:?} against {syntonic:?}",
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

/// Which nodes the Show row names, asked of a lattice at REST: nothing is
/// playing, nothing has been played and nothing is hovered, so whatever text
/// turns up is the mode's own answer rather than a note's or the pointer's.
///
/// The three differ only here. A sounding node and a hovered one are named
/// under all of them, which is what makes the resting picture the reading
/// that tells them apart.
#[test]
fn only_the_all_mode_names_a_node_nothing_has_happened_on() {
    let names_drawn = |names: harmonigraph_scene::NoteNames| -> Vec<String> {
        let mut state = fresh();
        state.view.show_labels = true;
        state.view.note_names = names;
        let scene = harmonigraph_scene::derive_scene(
            &state.tracker,
            &state.tuning,
            &state.view,
            &state.view.reach(),
            &state.frame_params,
            state.camera,
            None,
            0.0,
        );
        pane_labels(&scene, &state.view, 1.0)
            .pieces()
            .iter()
            .map(|piece| piece.text.clone())
            .collect()
    };

    for names in [harmonigraph_scene::NoteNames::Played, harmonigraph_scene::NoteNames::Past] {
        let drawn = names_drawn(names);
        assert!(drawn.is_empty(), "{names:?} named a node at rest: {drawn:?}");
    }
    // Named without a note or a memory behind it, which is the whole of what
    // All adds. The origin is in there by name -- the camera looks straight
    // at it -- and it is not alone, since the mode is about the field rather
    // than about one node.
    let all = names_drawn(harmonigraph_scene::NoteNames::All);
    assert!(all.iter().any(|text| text == "C"), "the origin went unnamed: {all:?}");
    assert!(all.len() > 1, "only one node was named: {all:?}");
}

/// A name and the marker under it always add up to ONE mark, measured END TO
/// END: the names come out of the label pass that really draws them and the
/// markers out of the scene the same frame was derived from.
///
/// Both read `NodeInstance::name_level`, so this cannot fail while they share
/// it — which is the point. What it guards is the day one of them stops: the
/// two live in different crates, one drawing type and one deriving geometry,
/// and a second spelling of the rule in either is a lattice that puts a marker
/// behind a name and looks merely smudged rather than wrong.
///
/// A SUM rather than "never both", because the handoff is a fade and the
/// middle of a fade is both: a name arriving at half strength stands over half
/// a marker, and it is the total that has to stay put. The strict reading held
/// only while the rule was a predicate, and it is what a note's release breaks
/// — see `a_marker_takes_back_what_a_names_fade_gives_up`, which measures the
/// same crossing from the scene side.
///
/// Swept over all three Show modes and over the Note names switch, because
/// each pair is a different balance of the two sets — All names everything and
/// leaves no markers, Played names nothing at rest and leaves them all, and the
/// switch off is the case where the mode says one thing and the picture
/// another.
#[test]
fn a_name_and_the_marker_under_it_add_up_to_one_mark() {
    for show in [true, false] {
        for names in [
            harmonigraph_scene::NoteNames::All,
            harmonigraph_scene::NoteNames::Past,
            harmonigraph_scene::NoteNames::Played,
        ] {
            let mut state = fresh();
            state.view.show_labels = show;
            state.view.note_names = names;
            // Something sounding and something remembered, so every route to a
            // name is open at once: a live note, a memory under Past, and the
            // whole field under All.
            for (time, kind) in [
                (0.0, harmonigraph_core::NoteEventKind::On { velocity: 1.0 }),
                (1.0, harmonigraph_core::NoteEventKind::Off),
            ] {
                let event = harmonigraph_core::NoteEvent { time, channel: 0, note: 60, kind };
                state.tracker.handle_event(event);
            }
            state.tracker.prune(3.0, &harmonigraph_core::Envelope::default());
            state.tracker.handle_event(harmonigraph_core::NoteEvent::on(3.5, 0, 67, 1.0));
            let scene = harmonigraph_scene::derive_scene(
                &state.tracker,
                &state.tuning,
                &state.view,
                &state.view.reach(),
                &state.frame_params,
                state.camera,
                None,
                4.0,
            );
            // The label pass is gated on the switch by its CALLER, so mirror
            // that here rather than asking it to gate itself.
            let batch = if show {
                pane_labels(&scene, &state.view, 1.0)
            } else {
                crate::text::TextBatch::default()
            };
            let named: Vec<(glam::Vec3, f32)> = batch
                .labels()
                .iter()
                .map(|label| {
                    let node = &scene.nodes[label.node as usize];
                    (node.world_pos, node.name_level(&state.view))
                })
                .collect();
            let ground = scene.lattice_ground.w;
            for (pos, level) in &named {
                let standing = scene
                    .pluses
                    .iter()
                    .find(|marker| marker.pos == *pos)
                    .map_or(0.0, |marker| marker.strength);
                let want = ground * (1.0 - level);
                assert!(
                    (standing - want).abs() < 1e-5,
                    "{names:?} (names on: {show}) at {pos:?}: a name at {level} \
                     over a marker at {standing}, which wanted {want}",
                );
            }
            // The half of it a sum cannot say on its own: a name at full
            // strength leaves NO instance, rather than one shipped at zero.
            for (pos, _) in named.iter().filter(|(_, level)| *level >= 1.0) {
                assert!(
                    !scene.pluses.iter().any(|marker| marker.pos == *pos),
                    "{names:?} (names on: {show}) shipped a marker under a whole name at {pos:?}",
                );
            }
            // And the sweep has to actually reach both sets, or it is passing
            // on an empty picture.
            match (show, names) {
                (true, harmonigraph_scene::NoteNames::All) => {
                    assert!(!named.is_empty() && scene.pluses.is_empty(), "All: {names:?}");
                }
                _ => assert!(!scene.pluses.is_empty(), "no pluses to contradict: {names:?}"),
            }
        }
    }
}

/// Every label the lattice draws, at one camera and one Size bar setting: the
/// rasterized type size, and the ink it actually covers.
fn lattice_labels_at(label_scale: f32, distance: f32, ppp: f32) -> Vec<(f32, egui::Rect)> {
    let mut state = fresh();
    state.view.show_labels = true;
    state.view.label_scale = label_scale;
    state.camera.distance = distance;
    // No arrival ramp: the scene below is derived at time 0, which under a
    // real Fade is the one instant the note has not lit yet, and a label
    // follows what its node is doing. This suite is about where a label is
    // DRAWN, not when it appears.
    state.frame_params.fade_time = 0.0;
    // Middle C: the origin node, which the camera looks straight at.
    state.tracker.handle_event(harmonigraph_core::NoteEvent::on(0.0, 0, 60, 1.0));
    let scene = harmonigraph_scene::derive_scene(
        &state.tracker,
        &state.tuning,
        &state.view,
        &state.view.reach(),
        &state.frame_params,
        state.camera,
        None,
        0.0,
    );

    let batch = pane_labels(&scene, &state.view, ppp);
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
    let tallest =
        |v: &[(f32, egui::Rect)]| v.iter().map(|(_, ink)| ink.height()).fold(0.0, f32::max);
    let (a, b) = (tallest(&just_past), tallest(&far_past));
    assert!(
        (a - b).abs() <= a * 0.01,
        "the Name size bar kept growing the drawn label past the raster ceiling: \
         {a} then {b}, a factor of {:.2} stretched out of one bitmap",
        b / a,
    );
}

/// The drawn marks follow the zoom the letters follow.
///
/// A label is rasterized at one size and DRAWN at another, and the comma marks
/// take the same split as the type: they are bitmaps on a whole-pixel grid, so
/// rasterizing them per zoom would step while the name beside them glided.
///
/// One route now rather than two — a mark is an instance of the same batch,
/// magnified by the same transform — which makes this test cheaper to satisfy
/// and no less worth keeping: what it pins is that a mark goes THROUGH that
/// transform at all. Emit one outside the `magnified` scope, or from a second
/// batch, and nothing else in the suite moves, because every other fixture
/// here draws at magnification 1. On screen it is the letters gliding smoothly
/// through a zoom while the `+` beside them jumps to the wrong side of the
/// letter and the wrong size.
///
/// Asserted as an identity rather than against measured coordinates: at
/// magnification `k` the whole label, text and marks alike, is its unmagnified
/// self scaled about the anchor by `k`. Measured numbers would only restate the
/// batch's own arithmetic and pass with it however it is written.
#[test]
fn the_drawn_marks_magnify_with_the_letters_beside_them() {
    let anchor = egui::pos2(200.0, 200.0);
    // Both kinds of drawn mark, and a count beside one of them: a name with no
    // comma draws no shapes at all, and then every assertion below holds over
    // an empty list and the test passes for free.
    let name = harmonigraph_core::NoteName {
        letter: 'B',
        sharps: -1,
        syntonic_commas: 2,
        septimal_commas: -1,
    };
    let (texts, shapes, _) = label_pieces(name, anchor, 1.0, 1.0);
    assert!(!shapes.is_empty(), "the fixture has to carry drawn marks to say anything");

    // About the anchor -- the point `TextBatch::magnified` is handed, so that
    // the label grows about the node it names rather than sliding off it.
    let scaled = |r: egui::Rect, k: f32| {
        egui::Rect::from_min_max(anchor + (r.min - anchor) * k, anchor + (r.max - anchor) * k)
    };
    // A shade under a tenth of a point: far below anything the eye or the
    // layout cares about, and far above the float noise of scaling a corner.
    let close = |a: egui::Rect, b: egui::Rect| {
        (a.min - b.min).length() < 0.05 && (a.max - b.max).length() < 0.05
    };

    // Either side of 1, and one of them barely off it: a magnification close to
    // 1 is the ordinary case on screen (a pane at its dialled-in size sits a
    // fraction of a percent off the rasterized size), and it is also where a
    // wrong transform moves a mark by least.
    for k in [0.9, 1.03, 1.4] {
        let (texts_k, shapes_k, _) = label_pieces(name, anchor, k, 1.0);
        assert_eq!(texts_k.len(), texts.len(), "magnifying must not change what is drawn");
        assert_eq!(shapes_k.len(), shapes.len(), "...nor how many marks it takes");
        for ((_, ink, text), (_, ink_k, text_k)) in texts.iter().zip(&texts_k) {
            assert_eq!(text, text_k, "the pieces must line up to be compared");
            assert!(
                close(scaled(*ink, k), *ink_k),
                "{text:?} at {k}x: expected {:?}, drew {ink_k:?}",
                scaled(*ink, k),
            );
        }
        for (i, (shape, shape_k)) in shapes.iter().zip(&shapes_k).enumerate() {
            assert!(
                close(scaled(*shape, k), *shape_k),
                "mark {i} at {k}x: expected {:?}, drew {shape_k:?}",
                scaled(*shape, k),
            );
        }
    }
}

/// A drawn mark is rasterized in DEVICE pixels and drawn in POINTS, so its
/// quad is its bitmap's size divided by the panel's pixel ratio -- and that
/// division is the whole of what keeps a `+` the same physical size on a
/// Retina panel and on a 1x monitor.
///
/// Invert it and the marks come out four times too big on one of the two and
/// right on the other, while the letters beside them stay right either way,
/// because those go through egui's own text path and cross the same boundary
/// there. Which is a defect that hides: the plugin is used daily on a Retina
/// display, so the configuration most likely to be seen is the one least
/// likely to be reasoned about.
#[test]
fn a_drawn_mark_holds_its_size_in_points_at_every_pixel_density() {
    let anchor = egui::pos2(200.0, 200.0);
    // Both kinds of drawn mark, each with a count beside it: a name with no
    // comma draws no shapes, and then everything below holds vacuously.
    let name = harmonigraph_core::NoteName {
        letter: 'B',
        sharps: -1,
        syntonic_commas: 2,
        septimal_commas: -2,
    };
    // Three marks, one quad each. Pinned as a NUMBER rather than read off the
    // list, so a mark that stopped being drawn at one density is a failure
    // rather than a shorter list quietly compared against itself.
    const MARKS: usize = 3;
    let (_, shapes, _) = label_pieces(name, anchor, 1.0, 1.0);
    assert_eq!(shapes.len(), MARKS, "the fixture has to draw all three marks: {shapes:?}");

    // What the raster ladder can move a box by at one density, in points. The
    // bitmap is a whole number of device pixels (`mark_geometry` ceils) around
    // a shape whose size was itself rounded to a whole pixel (`mark_key`) --
    // under 2.25 device pixels all told, which is that many points at ppp 1 and
    // half as many at ppp 2. A bound, not a measurement: the widest gap these
    // densities actually open is 0.7pt, on boxes 7 and 11 points across.
    let slack = |ppp: f32| 2.25 / ppp;

    // 2 is the Retina panel and the one that matters; 1.5 and 3 are there
    // because the ladder is not a scaling of the ppp-1 rung and a bug that
    // happened to divide evenly at 2 would sail through on its own.
    for ppp in [1.5, 2.0, 3.0] {
        let (_, at_ppp, _) = label_pieces(name, anchor, 1.0, ppp);
        assert_eq!(at_ppp.len(), MARKS, "a mark collapsed at ppp {ppp}: {at_ppp:?}");
        for (i, (one, dense)) in shapes.iter().zip(&at_ppp).enumerate() {
            let room = slack(1.0) + slack(ppp);
            assert!(
                (one.width() - dense.width()).abs() < room
                    && (one.height() - dense.height()).abs() < room,
                "mark {i} at ppp {ppp}: {:?} against {:?} at ppp 1, past {room} points",
                dense.size(),
                one.size(),
            );
            // The quad's PLACE crosses no such boundary -- it is the label's
            // layout, in points throughout -- so it holds to within the glyph
            // rounding that moves the column the mark hangs off.
            assert!(
                (one.center() - dense.center()).length() < 0.25,
                "mark {i} moved at ppp {ppp}: {:?} against {:?}",
                dense.center(),
                one.center(),
            );
        }
    }
}

/// What a label reports as its reach is the bottom of the mark it drew.
///
/// The reach is the caller's only view of a drawn mark: the cents readout is
/// placed off it, and a mark carries no text to be found by. So the number and
/// the quad are two readings of one bitmap that have to agree -- the same
/// `h / ppp` from opposite ends, and nothing else in the label compares them.
///
/// They agree on the INK, which is the quad less the clear margin every mark
/// bitmap carries ([`MARK_BITMAP_PAD`](marks::MARK_BITMAP_PAD)). The
/// readout is placed to clear the mark, and a margin is not something to
/// clear: reported off the quad instead, every cents line in the app would
/// hang a pixel lower than the ink it is getting out of the way of.
#[test]
fn the_reach_a_label_reports_is_where_its_drawn_mark_ends() {
    let anchor = egui::pos2(200.0, 200.0);
    // One bare comma: no count beside it, so the mark's own bitmap is the
    // lowest thing in the label and the reach is reporting it rather than a
    // digit's ink.
    let name = harmonigraph_core::NoteName {
        letter: 'B',
        sharps: 0,
        syntonic_commas: 1,
        septimal_commas: 0,
    };
    for ppp in [1.0, 2.0] {
        let (_, marks, reach) = label_pieces(name, anchor, 1.0, ppp);
        // The QUAD, which is the mark's bitmap: the halo the shader paints
        // around it reaches further and is one the readout is meant to overlap,
        // the way it overlaps a glyph's.
        let [mark] = marks[..] else { panic!("one mark at ppp {ppp}: {marks:?}") };
        let ink_bottom = mark.bottom() - marks::MARK_BITMAP_PAD as f32 / ppp;
        assert!(
            (anchor.y + reach - ink_bottom).abs() < 0.01,
            "at ppp {ppp} the label reports reaching {:.3} where its mark ends at {:.3}",
            anchor.y + reach,
            ink_bottom,
        );
    }
}

/// The cents readout hangs off the note name's GLYPHS, not its galley box --
/// a monospace line box carries several pixels of leading below the letter,
/// and spacing box-to-box left the readout visibly adrift from the name it
/// belongs to.
#[test]
fn the_cents_readout_sits_right_under_the_note_name() {
    let mut state = fresh();
    state.view.show_labels = true;
    state.view.show_cents = true;
    // Derived at time 0, so the note has to be lit without waiting on the
    // Fade's arrival — this is about where the readout SITS.
    state.frame_params.fade_time = 0.0;
    // Middle C: the origin node, which the default camera looks straight at.
    state.tracker.handle_event(harmonigraph_core::NoteEvent::on(0.0, 0, 60, 1.0));
    let scene = harmonigraph_scene::derive_scene(
        &state.tracker,
        &state.tuning,
        &state.view,
        &state.view.reach(),
        &state.frame_params,
        state.camera,
        None,
        0.0,
    );

    let batch = pane_labels(&scene, &state.view, 1.0);

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
    let scale = batch.pieces().iter().map(|p| p.font_size).fold(0.0, f32::max) / marks::NAME_SIZE;
    // Letter and marks together: the readout has to clear the comma, which
    // hangs lower than the letter does.
    let names = ink_of(&[marks::NAME_SIZE * scale, marks::MARK_SIZE * scale]);
    let cents = ink_of(&[marks::CENTS_SIZE * scale]);
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
        // above are built out of the batch's TEXT pieces, and a mark carries no
        // text. It is a fraction of the type, so the slack is quoted against
        // the gap rather than in absolute points — the regression this is
        // watching for, hanging the readout off the galley box instead of the
        // ink, is worth twice the gap and clears any of this.
        let want = marks::CENTS_GAP * scale;
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
/// range is not visible there and `sanitize` clamps to a written-out
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

/// Every label reaches the lattice attached to the node it names, and in the
/// pane's own space.
///
/// Both halves are what the lattice's callback is owed, and neither shows up
/// in the picture as anything but a name in the wrong place. The NODE is how
/// the callback decides where in its back-to-front order a name is drawn — it
/// sorts and culls the nodes itself, so an index off by one hands a name the
/// depth of its neighbour. And the SPACE is the pane's, because the pass a
/// name is drawn in is the pane's own: a label collected in screen points and
/// handed over unshifted lands right in a lattice docked at the window's
/// corner and further out the further the pane is from it.
#[test]
fn every_label_names_its_own_node_in_the_panes_own_space() {
    let mut state = fresh();
    state.view.show_labels = true;
    // Derived at time 0, so the notes have to be lit without waiting on the
    // Fade's arrival — this is about which name lands on which node.
    state.frame_params.fade_time = 0.0;
    // Two notes a third apart, so there are two names to tell apart and two
    // nodes to confuse them between.
    for note in [60, 64] {
        state.tracker.handle_event(harmonigraph_core::NoteEvent::on(0.0, 0, note, 1.0));
    }
    let scene = harmonigraph_scene::derive_scene(
        &state.tracker,
        &state.tuning,
        &state.view,
        &state.view.reach(),
        &state.frame_params,
        state.camera,
        None,
        0.0,
    );

    // The pane, and the same pane sitting at the window's corner: what the
    // callback is handed has to be the same picture either way.
    let labels_in = |min: egui::Pos2| -> harmonigraph_render::LatticeLabels {
        let rect = egui::Rect::from_min_size(min, egui::vec2(600.0, 500.0));
        lattice_labels_in(rect, &scene, &state)
    };
    let labels = labels_in(egui::pos2(120.0, 60.0));
    // Two notes light a node apiece on every sheet the view is showing, so
    // the count is a floor rather than a number: what is checked below is
    // that each of them names a node of its own, and the right one.
    assert!(labels.labels.len() >= 2, "two sounding notes, at least two names");
    let named: std::collections::HashSet<u32> = labels.labels.iter().map(|l| l.node).collect();
    assert_eq!(named.len(), labels.labels.len(), "two labels naming one node: {:?}", labels.labels);
    assert_eq!(
        labels.labels.iter().map(|l| l.glyphs).sum::<u32>() as usize,
        labels.glyphs.len(),
        "the runs have to account for every glyph handed over, or they name the wrong ones",
    );

    // Each run's glyphs sit on the node its label names. Measured against the
    // projection the callback's own sort works from, in the pane's space.
    let projector = harmonigraph_scene::Scene::projector(&scene, glam::Vec2::new(600.0, 500.0));
    let mut cursor = 0usize;
    for label in &labels.labels {
        let node = scene
            .nodes
            .get(label.node as usize)
            .unwrap_or_else(|| panic!("label {label:?} names no node of the scene"));
        assert!(node.activation > 0.0, "an unlit node has no name to draw: {label:?}");
        let ink = labels.glyphs[cursor..cursor + label.glyphs as usize]
            .iter()
            .map(|g| {
                egui::Rect::from_min_size(
                    egui::pos2(g.rect[0], g.rect[1]),
                    egui::vec2(g.rect[2], g.rect[3]),
                )
            })
            .reduce(|a, b| a.union(b))
            .expect("a label with no glyphs is not collected");
        cursor += label.glyphs as usize;
        let on = projector.project(node.world_pos).expect("the node is in front of the camera");
        assert!(
            ink.expand(2.0).contains(egui::pos2(on.x, on.y)),
            "the name of node {} is drawn at {ink:?}, not on the node at {on:?}",
            label.node,
        );
    }

    // And the same pane at the window's corner draws the same picture: what
    // the callback is handed is the pane's own space, not the screen's.
    //
    // To within the rounding of the subtraction itself, which is what makes
    // this a tolerance rather than an equality: a label at the far corner is
    // placed at some hundreds of points and shifted back by the pane's own
    // origin, and a float carries about five decimal digits there. The
    // failure this is looking for is the whole pane's offset, a hundred
    // points of it.
    let moved = labels_in(egui::Pos2::ZERO);
    assert_eq!(moved.glyphs.len(), labels.glyphs.len(), "the same pane draws the same glyphs");
    let off = moved
        .glyphs
        .iter()
        .zip(&labels.glyphs)
        .find(|(a, b)| a.rect.iter().zip(&b.rect).any(|(a, b)| (a - b).abs() > 1e-3));
    assert!(
        off.is_none(),
        "a label's place in its pane cannot depend on where the pane is: {off:?}",
    );
}

/// A name's drawn marks go into the run that names its own node, so whatever
/// covers the name covers them.
///
/// This is #207 from the collecting end, and the run is the whole mechanism:
/// the lattice's callback splices a label into the node draw order by its
/// `Label`, so a mark emitted outside that run — on the painter, in a second
/// batch, or simply after the `attached_to` scope closed — is a sign left
/// floating on the disc that just cut the letter beside it.
///
/// A held C sharp, so every name in the picture carries an accidental: the
/// home sheet spells it with one, and the sheets either side add a septimal
/// mark of their own. Both halves are asserted, and the first is what stops
/// the second passing over an empty list.
#[test]
fn a_names_drawn_marks_go_into_its_own_nodes_run() {
    let mut state = fresh();
    state.view.show_labels = true;
    state.frame_params.fade_time = 0.0;
    state.tracker.handle_event(harmonigraph_core::NoteEvent::on(0.0, 0, 61, 1.0));
    let scene = harmonigraph_scene::derive_scene(
        &state.tracker,
        &state.tuning,
        &state.view,
        &state.view.reach(),
        &state.frame_params,
        state.camera,
        None,
        0.0,
    );

    let labels = lattice_labels_in(PANE, &scene, &state);
    assert!(!labels.labels.is_empty(), "a held C sharp has to be named somewhere");

    let is_mark = |g: &harmonigraph_render::GlyphInstance| {
        g.atlas == harmonigraph_render::GlyphInstance::MARK
    };
    assert!(
        labels.glyphs.iter().any(is_mark),
        "no drawn mark reached the pass at all, so the run assertion below says nothing",
    );
    let mut cursor = 0usize;
    for label in &labels.labels {
        let run = &labels.glyphs[cursor..cursor + label.glyphs as usize];
        cursor += label.glyphs as usize;
        assert!(
            run.iter().any(is_mark),
            "the name of node {} carries no mark of its own: {} glyphs, none from the mark sheet",
            label.node,
            run.len(),
        );
    }
    assert_eq!(cursor, labels.glyphs.len(), "every glyph handed over belongs to some run");
}

/// A lattice name reconstructs both axes its orbiting camera can move it
/// along, rather than inheriting the one-axis default used by stationary
/// chrome and a horizontally scrolling roll.
///
/// A real held name has to reach the callback: an empty batch also returns a
/// default `LatticeLabels`, whose `Across` value would make this fail without
/// proving that the live path carries the choice.
#[test]
fn lattice_names_reconstruct_both_axes_the_camera_moves() {
    let mut state = fresh();
    state.view.show_labels = true;
    state.frame_params.fade_time = 0.0;
    state.tracker.handle_event(harmonigraph_core::NoteEvent::on(0.0, 0, 61, 1.0));
    let scene = harmonigraph_scene::derive_scene(
        &state.tracker,
        &state.tuning,
        &state.view,
        &state.view.reach(),
        &state.frame_params,
        state.camera,
        None,
        0.0,
    );
    let labels = lattice_labels_in(PANE, &scene, &state);
    assert!(!labels.glyphs.is_empty(), "the held C sharp must reach the lattice callback");
    assert_eq!(
        labels.slide,
        harmonigraph_render::SlideAxis::Both,
        "lattice names must reconstruct both axes an orbiting camera moves them along",
    );
}
