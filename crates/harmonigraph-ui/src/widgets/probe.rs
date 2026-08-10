//! What a widget's tests read a paint list back with.
//!
//! Every bar here is checked by painting it and asking where its parts came
//! out, and the questions are the same ones whichever bar it is: where the
//! fills are, where the text runs are, which of those runs is a knockout, and
//! which of the fills is a thumb. Shared rather than restated per widget,
//! because a second copy of a reader is a second answer to those questions the
//! day one of the two is taught something.

use egui::Ui;

use super::bar::HANDLE_W;
use crate::theme;

/// Paint `add` into a themed context of `size`, and answer the shapes it
/// emitted, each still carrying the clip rect it was painted through.
///
/// A context of its own per call rather than one held across a sweep: egui's
/// temp store has no expiry and a widget remembers what a drag had hold of in
/// it, so a shared context is one fixture reading the frame before it.
///
/// A bare context is the design scale, 1.0, which is why the geometry in these
/// tests reads the constants unmultiplied.
pub(super) fn painted_in(
    size: egui::Vec2,
    add: impl FnMut(&mut Ui),
) -> Vec<egui::epaint::ClippedShape> {
    let ctx = egui::Context::default();
    theme::apply_theme(&ctx);
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), size);
    ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, add).shapes
}

/// The same in a row `width` points across, which is the shape of nearly every
/// question here: a settings pane hands a bar the width of its column, and the
/// height is only ever room for the row it stands in.
pub(super) fn painted(width: f32, add: impl FnMut(&mut Ui)) -> Vec<egui::epaint::ClippedShape> {
    painted_in(egui::vec2(width, 100.0), add)
}

/// The same, as bare shapes — what every reading but the clip wants.
pub(super) fn shapes(width: f32, add: impl FnMut(&mut Ui)) -> Vec<egui::Shape> {
    painted(width, add).into_iter().map(|s| s.shape).collect()
}

/// The filled rects, in paint order.
pub(super) fn filled_rects(shapes: &[egui::Shape]) -> Vec<(egui::Rect, egui::Color32)> {
    shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Rect(r) if r.fill != egui::Color32::TRANSPARENT => {
                Some((r.rect, r.fill))
            }
            _ => None,
        })
        .collect()
}

/// The text runs and the boxes they occupy, in paint order.
///
/// One entry per RUN, which is not the same as one per pass: [`grip_over_text`]
/// repaints a run that a thumb stands in, at the same origin and with the
/// same string, to knock it out through the grip. That is a second pass over
/// a run already counted, so counting it here would say a bar draws two
/// names — and a test asking for "the last run" would get a fragment of one
/// clipped to 6pt. An override colour is what marks a knockout, and nothing
/// else in this module sets one.
pub(super) fn text_boxes(shapes: &[egui::Shape]) -> Vec<(egui::Rect, String)> {
    shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) if t.override_text_color.is_none() => Some((
                egui::Rect::from_min_size(t.pos, t.galley.size()),
                t.galley.text().to_owned(),
            )),
            _ => None,
        })
        .collect()
}

/// The knockout passes [`grip_over_text`] adds, in paint order, as
/// `(clip rect, the box of the run being knocked out, its string, colour)`.
///
/// Reads CLIPPED shapes, because the clip is the whole mechanism: a
/// knockout repeats its run's own galley at its own origin, so string, box
/// and position are all shared with the pass it doubles and only the clip
/// says it is confined to a thumb rather than painted over the whole run.
pub(super) fn knockouts(
    shapes: &[egui::epaint::ClippedShape],
) -> Vec<(egui::Rect, egui::Rect, String, Option<egui::Color32>)> {
    shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.override_text_color.is_some() => Some((
                s.clip_rect,
                egui::Rect::from_min_size(t.pos, t.galley.size()),
                t.galley.text().to_owned(),
                t.override_text_color,
            )),
            _ => None,
        })
        .collect()
}

/// The two handles, left to right.
pub(super) fn handles(shapes: &[egui::Shape]) -> Vec<egui::Rect> {
    let mut hs: Vec<_> = filled_rects(shapes)
        .into_iter()
        .filter(|(r, fill)| *fill == theme::text() && r.width() <= HANDLE_W + 0.01)
        .map(|(r, _)| r)
        .collect();
    hs.sort_by(|a, b| a.left().total_cmp(&b.left()));
    hs
}

pub(super) fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// What a galley actually PUTS ON SCREEN. `Galley::text()` answers with the
/// source string, so it cannot see an elision; the glyphs can.
pub(super) fn painted_text(galley: &egui::Galley) -> String {
    galley.rows.iter().flat_map(|row| row.glyphs.iter()).map(|g| g.chr).collect()
}
