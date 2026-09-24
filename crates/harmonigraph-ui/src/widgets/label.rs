//! Text laid out by its ink: a box from the top of its capitals to its last
//! baseline, where egui's label takes the whole line box.
//!
//! A line box carries the font's leading above the capitals, about a third of
//! the text's height, and a bar or a button carries none — its fill IS its box.
//! So one gap between boxes reads as two different gaps to the eye, depending
//! on which kind sits below it: a heading over a label looked half as far
//! again from it as a heading over a bar. Trimmed to its capitals, a label's
//! box is what shows, and a gap means the same thing whatever it separates.
//! This is what CSS `text-box-trim: cap alphabetic` and Figma's vertical trim
//! do, which egui has no setting for.
//!
//! Descenders hang below the box into the gap under it, as they do in CSS.

use egui::{Response, Sense, Ui, WidgetText};

/// How far below the top of its line box a galley's capitals start, and how
/// far above its bottom its last baseline sits: the two lengths [`label`]
/// trims off. Zero for a galley with no glyphs.
pub(crate) fn cap_trim(ui: &Ui, galley: &egui::Galley) -> (f32, f32) {
    let (Some(first), Some(last)) = (galley.rows.first(), galley.rows.last()) else {
        return (0.0, 0.0);
    };
    let (Some(first_glyph), Some(last_glyph)) = (first.glyphs.first(), last.glyphs.first()) else {
        return (0.0, 0.0);
    };
    let Some(font) = galley.job.sections.first().map(|s| s.format.font_id.clone()) else {
        return (0.0, 0.0);
    };
    let cap = cap_height(ui, font);
    let top = first.pos.y + first_glyph.pos.y - cap;
    let bottom = galley.size().y - (last.pos.y + last_glyph.pos.y);
    (top.max(0.0), bottom.max(0.0))
}

/// The height of a capital above the baseline in `font`, read off the ink of
/// an "H". egui caches the layout, so asking every frame is a lookup.
fn cap_height(ui: &Ui, font: egui::FontId) -> f32 {
    let h = ui.fonts_mut(|fonts| fonts.layout_no_wrap("H".into(), font, egui::Color32::WHITE));
    match h.rows.first().and_then(|row| Some((row, row.glyphs.first()?))) {
        Some((row, glyph)) => row.pos.y + glyph.pos.y - h.mesh_bounds.min.y,
        None => 0.0,
    }
}

/// A label whose box is its capitals to its last baseline (see the module
/// docs). Wraps as `ui.label` does, and answers hovers for a tooltip.
///
/// Standing alone in a column it is a [group](group_space): [`GROUP_GAP`] from
/// whatever is over and under it. In a row beside a control it takes no room,
/// and egui centres its capitals on the control rather than its line box.
pub fn label(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    group_space(ui);
    let galley = text.into().into_galley(ui, None, ui.available_width(), egui::TextStyle::Body);
    let (top, bottom) = cap_trim(ui, &galley);
    let size = egui::vec2(galley.size().x, galley.size().y - top - bottom);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), galley.text())
    });
    if ui.is_rect_visible(rect.expand2(egui::vec2(0.0, top.max(bottom)))) {
        ui.painter().galley(rect.min - egui::vec2(0.0, top), galley, ui.visuals().text_color());
    }
    group_end(ui);
    response
}

/// How far a line of text or a fold standing alone in a column sits from
/// whatever is over and under it, ink to ink, at scale 1 — and a section
/// heading from its rule and its first row.
///
/// Every box sits where its ink is (see the module docs), so the row gap alone
/// would put a line of text as near a bar as two bars sit to each other, which
/// reads cramped. Bars and buttons keep the row gap between themselves.
pub(crate) const GROUP_GAP: f32 = 9.0;

fn spaced_id() -> egui::Id {
    egui::Id::new("group-spaced")
}

/// Record that the cursor where `ui` stands already has all the room over the
/// next thing it should: a section heading's gap, or a [`group_end`].
/// A [`group_space`] asked for right here adds nothing.
pub(crate) fn mark_spaced(ui: &Ui) {
    let at = ui.cursor().min;
    ui.data_mut(|d| d.insert_temp(spaced_id(), at));
}

/// Room over a group in a column, bringing the row gap up to [`GROUP_GAP`] —
/// unless the cursor is where [`mark_spaced`] said the room is already there.
/// The top of a child ui is no exception: an `add_enabled_ui` block starts one
/// wherever it stands in its section.
/// Nothing in a row, where space would run sideways; a grid lays its cells out
/// in rows, so nothing there either.
pub(crate) fn group_space(ui: &mut Ui) {
    if !ui.layout().is_vertical() {
        return;
    }
    let at = ui.cursor().min;
    let marked = ui.data(|d| d.get_temp::<egui::Pos2>(spaced_id()));
    if marked.is_some_and(|m| m.distance(at) < 0.5) {
        return;
    }
    ui.add_space(extra(ui));
}

/// Room under a group in a column, the same as [`group_space`] put over it,
/// marked so that a group right after it does not add its own on top.
pub(crate) fn group_end(ui: &mut Ui) {
    if !ui.layout().is_vertical() {
        return;
    }
    ui.add_space(extra(ui));
    mark_spaced(ui);
}

/// What a group adds to the row gap to stand [`GROUP_GAP`] off.
fn extra(ui: &Ui) -> f32 {
    (GROUP_GAP * crate::theme::ui_scale(ui.ctx()) - ui.spacing().item_spacing.y).max(0.0)
}

/// [`label`] in the weak text colour, for `ui.weak`.
pub fn weak(ui: &mut Ui, text: impl Into<egui::RichText>) -> Response {
    label(ui, text.into().weak())
}
