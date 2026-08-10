//! What a widget's tests read a paint list back with.
//!
//! Every bar here is checked by painting it and asking where its parts came
//! out, and the questions are the same ones whichever bar it is: where the
//! fills are, where the text runs are, which of those runs is a knockout, and
//! which of the fills is a thumb. Shared rather than restated per widget,
//! because a second copy of a reader is a second answer to those questions the
//! day one of the two is taught something.

use super::bar::HANDLE_W;
use crate::theme;

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
