//! [`ValueBar`], the workhorse — and [`progress_bar`], the same shape read-only.

use std::ops::RangeInclusive;

use egui::{Color32, CornerRadius, Key, Response, Sense, TextEdit, TextStyle, Ui, Vec2};

use super::bar::{bar_radius, bar_width, elided_name, track_fill, BAR_TEXT_PAD};
use crate::theme;

/// How many segments a [`ValueBar::curve`] preview is drawn in.
///
/// The preview is one bend and no more, so the count only has to keep that
/// bend from reading as a corner — 64 puts a vertex every few points at the
/// widest a settings column opens to, and the sharpest curve on offer spends
/// a dozen of them on its knee.
const CURVE_SEGMENTS: usize = 64;

// Both lengths below are the bars' own geometry, written at the design size and
// multiplied by `theme::ui_scale` where they are drawn — the contract
// `super::bar` states for the lengths it holds. The segment count above is not
// one of them: how finely the line is sampled is the same at every size.

/// How far a [`ValueBar::curve`] preview is held off the top and bottom of the
/// track, so the flat ends of the curve read as a line rather than as the
/// track's own rim.
const CURVE_INSET: f32 = 3.5;

/// Thickness of the preview line.
const CURVE_WIDTH: f32 = 1.5;

/// The preview line's color: the dim text, softened again.
///
/// It crosses both halves of the track — accent fill up to the value, bare
/// well past it — so it cannot be a color that only reads on one of them. Dim
/// on purpose beyond that: the line is a PICTURE of what the number means and
/// the number itself is the reading, so it stays under the two text runs it
/// passes behind rather than competing with them.
pub(crate) fn curve_color() -> Color32 {
    theme::text_dim().gamma_multiply(0.55)
}

/// Every preview line in `shapes`, each as the points it was drawn through, in
/// paint order.
///
/// Shared by the two places that ask: the bar's own tests, which check WHERE
/// the line is drawn, and the Lattice page's, which checks WHICH curve each
/// bar draws. The color is what identifies a line — nothing else in a settings
/// pane draws an open path in it.
#[cfg(test)]
pub(crate) fn curve_paths(shapes: &[egui::Shape]) -> Vec<Vec<egui::Pos2>> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Path(path)
                if path.stroke.color == egui::epaint::ColorMode::Solid(curve_color()) =>
            {
                Some(path.points.clone())
            }
            _ => None,
        })
        .collect()
}

/// The one preview line in `shapes`, for a fixture that paints one bar.
#[cfg(test)]
fn curve_points(shapes: &[egui::Shape]) -> Vec<egui::Pos2> {
    curve_paths(shapes).into_iter().flatten().collect()
}

pub struct ValueBar<'a> {
    value: &'a mut f32,
    range: RangeInclusive<f32>,
    label: &'a str,
    /// Ease the low end of the range (geometric when min > 0, cubic
    /// otherwise), so the fine end of a wide range is draggable.
    eased: bool,
    decimals: usize,
    integer: bool,
    /// A word saying what is DRIVING the value, drawn at full brightness
    /// ahead of the bar's name. Used by the bar of an axis a tempered-out
    /// comma derives (the major third under meantone, the harmonic seventh
    /// under marvel): the number in the bar is then not the one the param
    /// holds, and the badge is what says so.
    badge: Option<&'a str>,
    /// `(target, tolerance)`: a value the bar is pulled to while an edit
    /// lands within `tolerance` of it (see [`ValueBar::magnet`]).
    magnet: Option<(f32, f32)>,
    /// How the value READS OUT, when plain decimals won't say it (see
    /// [`ValueBar::display`]). Never how it is typed in.
    display: Option<fn(f32) -> String>,
    /// A picture of what the value MEANS, drawn across the track (see
    /// [`ValueBar::curve`]).
    curve: Option<fn(f32, f32) -> f32>,
}

impl<'a> ValueBar<'a> {
    pub fn new(value: &'a mut f32, range: RangeInclusive<f32>, label: &'a str) -> Self {
        ValueBar {
            value,
            range,
            label,
            eased: false,
            decimals: 2,
            integer: false,
            badge: None,
            magnet: None,
            display: None,
            curve: None,
        }
    }

    pub fn eased(mut self, on: bool) -> Self {
        self.eased = on;
        self
    }

    /// Mark the bar as driven from elsewhere, with `word` at the front of
    /// its name. The bar stays draggable: what drives it is a MODE, and the
    /// bar is where that mode is let go of.
    pub fn badge(mut self, word: &'a str) -> Self {
        self.badge = Some(word);
        self
    }

    /// Pull edits to `target` while they land within `tolerance` of it, so
    /// the bar holds the target exactly until a drag pulls clear of the
    /// window — the meantone third, held to four perfect fifths until it is
    /// dragged off them.
    ///
    /// Applied where the value is DECIDED, ahead of the paint, which is the
    /// whole point: a caller correcting the value afterwards would have to
    /// let the bar draw one frame at the pointer first, and a bar that
    /// follows the pointer for a frame and snaps back on the next is a bar
    /// that visibly does not snap while you drag it slowly.
    ///
    /// The value the caller gets back is snapped too, so "did this edit
    /// escape the magnet" is the same question as "did the value change".
    pub fn magnet(mut self, target: f32, tolerance: f32) -> Self {
        self.magnet = Some((target, tolerance));
        self
    }

    /// `v`, pulled to the magnet's target if it is inside the window.
    fn snapped(&self, v: f32) -> f32 {
        match self.magnet {
            Some((target, tolerance)) if (v - target).abs() <= tolerance => target,
            _ => v,
        }
    }

    pub fn decimals(mut self, n: usize) -> Self {
        self.decimals = n;
        self
    }

    pub fn integer(mut self) -> Self {
        self.integer = true;
        self.decimals = 0;
        self
    }

    /// Read the value out as this rather than as plain decimals — for a
    /// value whose UNIT changes with its size, which a fixed suffix in the
    /// label can't say (seconds that become minutes and seconds).
    ///
    /// Display only. Typing still takes a bare number in the bar's own
    /// units, and double-click seeds the box with one, so whatever a
    /// formatter does to the readout the value stays typeable.
    pub fn display(mut self, display: fn(f32) -> String) -> Self {
        self.display = Some(display);
        self
    }

    /// Draw what the value DOES across the track: `curve(value, p)` for `p`
    /// walking 0 to 1 gives the level reached that far through, and the line
    /// is those points. For the Note section's Fade curve bar, whose number names a
    /// curve and says nothing about its character — the difference between a
    /// straight line and a knee is the whole setting, and 0.35 does not carry
    /// it.
    ///
    /// **The curve's x is not the bar's own axis**, and it spans the whole
    /// track at every value because of that: the bar's x is where the value
    /// sits in its range, the curve's is progress through the transition the
    /// value shapes. Two rulers over one rectangle is what a preview drawn
    /// inside a control costs, and the alternative — a picture in a row of its
    /// own — costs the row instead. What keeps it readable is that the curve
    /// is drawn as a LINE and the value as a filled slice, so neither reads as
    /// a measurement of the other.
    ///
    /// Hand it the same function the thing being previewed runs on rather than
    /// a copy of the formula: a preview that drifts from what it previews is
    /// worse than none, and there is nothing on screen that would show the
    /// drift.
    pub fn curve(mut self, curve: fn(f32, f32) -> f32) -> Self {
        self.curve = Some(curve);
        self
    }

    fn min(&self) -> f32 {
        *self.range.start()
    }

    fn max(&self) -> f32 {
        *self.range.end()
    }

    /// Value -> fill fraction in [0, 1].
    fn to_t(&self, v: f32) -> f32 {
        let (min, max) = (self.min(), self.max());
        let v = v.clamp(min, max);
        if !self.eased {
            (v - min) / (max - min)
        } else if min > 0.0 {
            (v / min).ln() / (max / min).ln()
        } else {
            ((v - min) / (max - min)).cbrt()
        }
    }

    /// Fill fraction -> value, with integer snapping.
    fn value_at(&self, t: f32) -> f32 {
        let (min, max) = (self.min(), self.max());
        let t = t.clamp(0.0, 1.0);
        let v = if !self.eased {
            min + t * (max - min)
        } else if min > 0.0 {
            min * (max / min).powf(t)
        } else {
            min + t.powi(3) * (max - min)
        };
        if self.integer {
            v.round()
        } else {
            v
        }
    }

    /// The value as text to TYPE: always a bare number, so the text-entry
    /// box round-trips through `parse::<f32>` whatever the readout says.
    fn format(&self, v: f32) -> String {
        format!("{:.*}", self.decimals, v)
    }

    /// The value as text to READ.
    fn shown(&self, v: f32) -> String {
        match self.display {
            Some(display) => display(v),
            None => self.format(v),
        }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let scale = theme::ui_scale(ui.ctx());
        let width = bar_width(ui);
        let (rect, mut response) = ui.allocate_exact_size(
            Vec2::new(width, theme::row_height(scale)),
            Sense::click_and_drag(),
        );

        let edit_id = response.id.with("edit");
        let focus_id = edit_id.with("focus_pending");

        // ---- Text-entry mode (double-click) ---------------------------------
        if let Some(mut text) = ui.data(|d| d.get_temp::<String>(edit_id)) {
            let output = ui.put(
                rect,
                TextEdit::singleline(&mut text)
                    .font(TextStyle::Monospace)
                    .horizontal_align(egui::Align::Center),
            );
            // TextEdit never takes focus by itself; grab it on the first
            // edit-mode frame so typing (and focus-loss commit) works at
            // all.
            if ui.data(|d| d.get_temp::<bool>(focus_id)).unwrap_or(false) {
                output.request_focus();
                ui.data_mut(|d| d.remove_temp::<bool>(focus_id));
            }
            // Escape cancels; everything that drops focus (Enter included
            // — egui surrenders TextEdit focus on both) commits. Enter is
            // NOT checked globally: that would commit every bar in edit
            // mode at once, focused or not.
            let cancelled = ui.input(|i| i.key_pressed(Key::Escape));
            if cancelled || output.lost_focus() {
                if !cancelled {
                    if let Ok(v) = text.trim().parse::<f32>() {
                        // Reject NaN/inf: NaN survives clamp() and would
                        // poison the param (and the host's automation lane
                        // in the plugin) in a state the bar can't display
                        // or drag back out of.
                        if v.is_finite() {
                            let v = self.snapped(v.clamp(self.min(), self.max()));
                            *self.value = if self.integer { v.round() } else { v };
                            response.mark_changed();
                        }
                    }
                }
                ui.data_mut(|d| d.remove_temp::<String>(edit_id));
            } else {
                ui.data_mut(|d| d.insert_temp(edit_id, text));
            }
            return response;
        }

        // ---- Interaction ----------------------------------------------------
        if response.double_clicked() {
            ui.data_mut(|d| d.insert_temp(edit_id, self.format(*self.value)));
            ui.data_mut(|d| d.insert_temp(focus_id, true));
            return response;
        }
        // Drag-to-set only (no click-jump): a stray click can't yank a
        // carefully tuned parameter, and it can't fight the double-click.
        if response.dragged() {
            if let Some(pointer) = response.interact_pointer_pos() {
                let t = (pointer.x - rect.left()) / rect.width().max(1.0);
                let new_value = self.snapped(self.value_at(t));
                if new_value != *self.value {
                    *self.value = new_value;
                    response.mark_changed();
                }
            }
        }

        // ---- Paint ----------------------------------------------------------
        let radius = CornerRadius::same(bar_radius(scale));
        let painter = ui.painter();
        painter.rect_filled(rect, radius, theme::well());

        let t = self.to_t(*self.value);
        let fill_color = track_fill(&response);
        let mut fill = rect;
        fill.set_width(rect.width() * t);
        painter.rect_filled(fill, radius, fill_color);

        // Over the fill and under the text: the fill is what the curve is
        // drawn ON and the two text runs are what it is drawn UNDER, which is
        // the order that keeps the number legible where the line passes behind
        // it (`a_curve_preview_is_drawn_over_the_fill_and_under_the_bars_own_text`).
        // The fill half is not a nicety — an accent mix is fully opaque, so a
        // line drawn under it is erased rather than dimmed.
        if let Some(curve) = self.curve {
            let plot = rect.shrink2(Vec2::new(BAR_TEXT_PAD * scale, CURVE_INSET * scale));
            let points = (0..=CURVE_SEGMENTS)
                .map(|i| {
                    let p = i as f32 / CURVE_SEGMENTS as f32;
                    // Clamped because the caller's function is the real one and
                    // not a drawing routine: it answers for the thing being
                    // previewed, and a level off either end would draw over the
                    // bars above and below rather than being cut off here.
                    let level = curve(*self.value, p).clamp(0.0, 1.0);
                    egui::pos2(
                        plot.left() + plot.width() * p,
                        plot.bottom() - plot.height() * level,
                    )
                })
                .collect::<Vec<_>>();
            painter.add(egui::Shape::line(
                points,
                egui::Stroke::new(CURVE_WIDTH * scale, curve_color()),
            ));
        }

        let text_color = if response.hovered() || response.dragged() {
            theme::text()
        } else {
            theme::text_dim()
        };
        // The value is laid out first and the name takes what is left, elided.
        // The number is what the bar is FOR — a name that runs over it, or out
        // past the pane edge, costs the reading the control exists to give.
        // Values in monospace: digits align and don't wiggle as they
        // change.
        let mono = TextStyle::Monospace.resolve(ui.style());
        let value = painter.layout_no_wrap(self.shown(*self.value), mono.clone(), theme::text());
        // Room kept clear for the readout, measured from the widest one the
        // bar's RANGE can produce rather than from the number currently in it.
        // Taking it from the current number makes the name re-elide the moment
        // the value gains a digit — the name wobbling under the pointer
        // mid-drag, which is exactly what the monospace face buys for the
        // digits themselves. The ends bound it for a plain decimal readout;
        // the current value is in the max as well so that a `display` whose
        // length is not monotonic in the value can still never be overlapped.
        let reserve = [self.shown(self.min()), self.shown(self.max()), self.shown(*self.value)]
            .into_iter()
            .map(|text| painter.layout_no_wrap(text, mono.clone(), theme::text()).size().x)
            .fold(0.0f32, f32::max);
        // A badged bar wears the word at the FRONT of its name, because that is
        // the end elision cannot reach. Spelled into the tail it is the first
        // thing dropped in a narrow column — and the bar then reads as an
        // ordinary one that ran out of room, which is worse than saying
        // nothing, since the unbadged name is the shorter of the two and draws
        // in full at the same width. Here rather than in the caller's label so
        // every badged bar gets it, and so the name is the same string badged
        // or not. Full brightness while the name beside it is dim: the badge
        // is state, not more label.
        let body = TextStyle::Body.resolve(ui.style());
        let mut job = egui::text::LayoutJob::default();
        if let Some(word) = self.badge {
            let format = egui::TextFormat::simple(body.clone(), theme::text());
            job.append(&format!("{word} · "), 0.0, format);
        }
        job.append(self.label, 0.0, egui::TextFormat::simple(body, text_color));
        let text_pad = BAR_TEXT_PAD * scale;
        let label = elided_name(painter, job, rect.width(), scale, reserve);
        let centered =
            |galley: &egui::Galley, x: f32| egui::pos2(x, rect.center().y - galley.size().y * 0.5);
        painter.galley(centered(&label, rect.left() + text_pad), label, text_color);
        painter.galley(
            centered(&value, rect.right() - text_pad - value.size().x),
            value,
            theme::text(),
        );

        response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
    }
}

// ---- The sweep that crosses a running bar ----------------------------------
// A run of the FILL, drawn a shade up, travelling from one end of it to the
// other: the bar saying it is running, in the only two things a settings pane
// is built out of — a flat rect and a colour off the skin.
//
// Flat is the constraint, not an economy. Nothing else in a settings pane is
// drawn in a gradient: the wells, the fills, the buttons and the handles are
// each one opaque colour, and the two bands that are not (a
// [`SpectrumBar`](super::gradient::SpectrumBar) track, a
// [`GradientPreview`](super::gradient::GradientPreview)) are gradients because
// the value they draw IS a ramp of colour. A soft-edged sweep here would be the
// one shading in the pane that means nothing.

/// Seconds the sweep takes to cross the filled part once.
///
/// Paced against what the bar is measuring: a render is minutes of work, so it
/// has to say "running" to a glance without asking to be watched. Two glances a
/// second apart find it somewhere else, and nothing about it reads as a
/// countdown — which a period near a second would, the eye taking a beat that
/// fast as one tick per unit of something.
///
/// A period rather than a speed, so what it crosses is always the filled part
/// however long that is. A fixed speed would whip across a bar a twentieth
/// full several times a second and crawl the same distance on a full one, which
/// is the animation running at a rate the value sets.
const SWEEP_PERIOD: f64 = 2.4;

/// How much of the filled part the sweep covers at once, as a share of it.
///
/// A share of the FILL and not of the track, so the sweep is the same picture
/// at every fraction: a width off the track would be most of a short fill and a
/// sliver of a long one, which is a second thing the bar's value silently
/// changes about it.
const SWEEP_SHARE: f32 = 0.22;

/// A read-only bar reporting how far something running in the background has
/// got: `fraction` of the track filled, `label` on the left and `value` read
/// out on the right.
///
/// The same shape as [`ValueBar`] — same track, fill, and text placement — so
/// a pane reads as one kind of control whether the number is one you set or
/// one you are being told. It senses hover only: there is nothing here to
/// drag, and a bar that moved under the pointer would claim there was.
///
/// **A sweep crosses the FILLED part while it stands**, on the clock rather
/// than on the value, so the bar says it is running as well as how far along it
/// is. Those are two jobs and only the second is a number: a bar can go a long
/// while without its fill visibly moving — a render counting to five thousand
/// frames advances the edge by a twentieth of a point a frame — and a still bar
/// and a hung one look alike.
///
/// It is confined to the fill, which is what keeps it from being readable as a
/// second value. Everything past the frontier is what has NOT happened, and a
/// pane drawing on it, however faintly, is drawing on the part of the bar whose
/// whole meaning is that nothing is there yet.
///
/// `fraction` is `None` while the total is still unknown, and then the track
/// draws EMPTY rather than at zero — "no idea yet" and "none of it done" are
/// different things, and only one of them is a number. There is no fill to
/// sweep in that state and nothing animates: what says a render is alive before
/// its first frame count is the status line naming the file it is writing.
///
/// Nothing keeps the name from re-eliding as the readout grows, unlike
/// `ValueBar`, which reserves the width of the widest value its range can
/// reach. There is no range to ask here, so a caller whose readout changes
/// width pads it to a fixed one instead (monospace, so that is enough).
pub fn progress_bar(ui: &mut Ui, fraction: Option<f32>, label: &str, value: &str) -> Response {
    let scale = theme::ui_scale(ui.ctx());
    let width = bar_width(ui);
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, theme::row_height(scale)), Sense::hover());

    let corner = bar_radius(scale);
    let radius = CornerRadius::same(corner);
    let fill = fraction.map(|t| {
        let mut fill = rect;
        fill.set_width(rect.width() * t.clamp(0.0, 1.0));
        fill
    });
    // The run of the fill the sweep covers this frame, decided before the
    // painter is borrowed and `None` when there is no fill to cross.
    //
    // Its leading edge runs from its own width behind the fill's left end to
    // the frontier, and what is drawn is the part of that inside the fill —
    // so it comes on at one end and goes off at the other rather than jumping,
    // which is the one motion here that would read as a glitch. Both edges are
    // hard: at the ends the fill's own boundary is what cuts it, and in the
    // middle it is a rect like everything else on the pane.
    //
    // The phase is the clock alone, so two bars on screen sweep together rather
    // than each from whenever it appeared, and the repaint is asked for only
    // where something is actually moving. Unconditional is the record button's
    // way and would be wrong here: this bar draws a still picture whenever the
    // total is unknown, and there is nothing to spend a frame on then.
    let sweep = fill.filter(|fill| fill.width() > 0.0).and_then(|fill| {
        let phase = (ui.ctx().input(|i| i.time) / SWEEP_PERIOD).rem_euclid(1.0) as f32;
        ui.ctx().request_repaint();
        let span = fill.width() * SWEEP_SHARE;
        let lead = fill.left() + (fill.width() + span) * phase;
        let run = egui::Rect::from_x_y_ranges(
            (lead - span).max(fill.left())..=lead.min(fill.right()),
            fill.y_range(),
        );
        (run.width() > 0.0).then_some((fill, run))
    });

    let painter = ui.painter();
    painter.rect_filled(rect, radius, theme::well());
    if let Some(fill) = fill {
        painter.rect_filled(fill, radius, theme::accent_fill());
    }
    if let Some((fill, run)) = sweep {
        // Rounded only where it stands on an end of the fill, so it wears that
        // end's own cap and is square wherever it cuts across. One radius for
        // both corners on a side: the fill is a rect with the shared control
        // radius on all four, and a sweep meeting it has to round to the same
        // arc or it draws a step inside a curve.
        let cap = |on: bool| if on { corner } else { 0 };
        let (head, tail) =
            (cap(run.right() >= fill.right() - 0.01), cap(run.left() <= fill.left() + 0.01));
        let ends = CornerRadius { nw: tail, sw: tail, ne: head, se: head };
        // The fill a shade up: a colour the pane already wears rather than a new
        // one, so the sweep cannot read as a state the bar has entered. Opaque,
        // like every accent mix in the skin — an alpha over the fill would be a
        // third colour that exists nowhere else in the panel.
        painter.rect_filled(run, ends, theme::accent_fill_hover());
    }

    // Value laid out first and the name elided into what is left, the order
    // and the reason `ValueBar` uses: the number is what the bar is for.
    let value = painter.layout_no_wrap(
        value.to_owned(),
        TextStyle::Monospace.resolve(ui.style()),
        theme::text(),
    );
    let job = egui::text::LayoutJob::simple_singleline(
        label.to_owned(),
        TextStyle::Body.resolve(ui.style()),
        theme::text_dim(),
    );
    let text_pad = BAR_TEXT_PAD * scale;
    let label = elided_name(painter, job, rect.width(), scale, value.size().x);
    let centered =
        |galley: &egui::Galley, x: f32| egui::pos2(x, rect.center().y - galley.size().y * 0.5);
    painter.galley(centered(&label, rect.left() + text_pad), label, theme::text_dim());
    painter.galley(
        centered(&value, rect.right() - text_pad - value.size().x),
        value,
        theme::text(),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::probe::{filled_rects, painted, painted_text, shapes, shapes_at};

    /// Paint one value bar across a `width`-point row and return what it
    /// emitted, with or without a preview curve.
    fn paint_value_bar(
        width: f32,
        value: f32,
        curve: Option<fn(f32, f32) -> f32>,
    ) -> Vec<egui::Shape> {
        let mut value = value;
        shapes(width, |ui| {
            let mut bar = ValueBar::new(&mut value, 0.0..=1.0, "Shape");
            if let Some(curve) = curve {
                bar = bar.curve(curve);
            }
            bar.show(ui);
        })
    }

    /// A straight line, which is what a preview of nothing in particular looks
    /// like: the tests below are about WHERE the curve is drawn, and the one
    /// that cares which curve it is samples the real bar in the real pane
    /// (`the_shape_bars_preview_is_the_curve_the_notes_run_on`).
    fn ramp(_value: f32, p: f32) -> f32 {
        p
    }

    /// The preview is a picture of the transition and not a second reading of
    /// the value, so it spans the whole track wherever the value sits — the two
    /// axes over one rectangle that [`ValueBar::curve`] is about. A preview
    /// that stopped at the fill would read as the part of the curve that has
    /// happened, which is a claim about time the bar cannot make.
    #[test]
    fn a_curve_preview_spans_the_track_at_every_value() {
        let ends = |value: f32| {
            let shapes = paint_value_bar(240.0, value, Some(ramp));
            let points = curve_points(&shapes);
            assert!(points.len() > 2, "a preview of {value} drew {} points", points.len());
            (points[0].x, points[points.len() - 1].x)
        };
        let empty = ends(0.0);
        for value in [0.35f32, 1.0] {
            assert_eq!(
                ends(value),
                empty,
                "the preview moved with the value, which makes its x the bar's own axis",
            );
        }
    }

    /// The line goes over the fill and under the text, and BOTH halves are the
    /// contract. Text over it, because the curve crosses the whole track and so
    /// passes behind both runs wherever they sit, and a bar whose number is
    /// hard to read is a bar that has given up what it is FOR to decorate
    /// itself. Fill under it, because the fill is opaque — every accent mix is,
    /// deliberately (see `theme`) — so a line painted first is not dimmed by it
    /// but erased, over the whole filled part of the track and over all of it
    /// at the top of the range.
    ///
    /// Z-order and not geometry, which is why this reads indices: both halves
    /// leave every point exactly where it was, so nothing that samples the
    /// line's own coordinates can see either one go wrong.
    #[test]
    fn a_curve_preview_is_drawn_over_the_fill_and_under_the_bars_own_text() {
        // Mid-range, so there IS a fill to be buried by and empty track past it.
        let shapes = paint_value_bar(240.0, 0.5, Some(ramp));
        let line = shapes
            .iter()
            .position(|shape| match shape {
                egui::Shape::Path(path) => {
                    path.stroke.color == egui::epaint::ColorMode::Solid(curve_color())
                }
                _ => false,
            })
            .expect("the bar painted no preview");
        // The fill and not the well: they are both rects and only one of them
        // is the one the line has to clear.
        let fill = shapes
            .iter()
            .position(|shape| match shape {
                egui::Shape::Rect(rect) => rect.fill == theme::accent_fill(),
                _ => false,
            })
            .expect("the bar painted no fill");
        assert!(fill < line, "the fill at {fill} was painted over the line at {line}");
        let texts: Vec<usize> = shapes
            .iter()
            .enumerate()
            .filter(|(_, shape)| matches!(shape, egui::Shape::Text(_)))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(texts.len(), 2, "the bar painted {} text runs", texts.len());
        for text in texts {
            assert!(text > line, "a text run at {text} was painted under the line at {line}");
        }
    }

    /// A caller whose function leaves 0..=1 is drawn inside the bar anyway.
    ///
    /// The guard exists because the function handed to [`ValueBar::curve`] is
    /// the REAL one — it answers for the thing being previewed and owes the
    /// widget nothing — so the widget cannot assume a level it can draw. The
    /// one caller in the tree is bounded by construction, which is exactly why
    /// this needs a fixture rather than a reader: without one the clamp can be
    /// deleted and the whole suite stays green.
    ///
    /// A bar is one row in a column of them, so the cost of getting this wrong
    /// is not a clipped line but ink over the neighbours' tracks.
    #[test]
    fn a_curve_preview_stays_inside_the_bar_when_its_caller_does_not() {
        // Runs -0.5 to 1.5: half a plot-height below the bar and half above.
        let shapes = paint_value_bar(240.0, 0.5, Some(|_, p| p * 2.0 - 0.5));
        let track = shapes
            .iter()
            .find_map(|shape| match shape {
                egui::Shape::Rect(rect) if rect.fill == theme::well() => Some(rect.rect),
                _ => None,
            })
            .expect("the bar painted no track");
        let points = curve_points(&shapes);
        assert!(!points.is_empty(), "the bar painted no preview");
        for point in &points {
            assert!(
                point.y >= track.top() && point.y <= track.bottom(),
                "a preview point at y {} left a track of {}..{}",
                point.y,
                track.top(),
                track.bottom(),
            );
        }
    }

    /// The preview is opt-in, and every other bar in the tree is the bar it
    /// always was: no line, and no extra shape for the paint tests around it to
    /// trip over.
    #[test]
    fn a_bar_with_no_curve_paints_no_line() {
        let shapes = paint_value_bar(240.0, 0.35, None);
        assert!(curve_points(&shapes).is_empty(), "a plain bar painted a preview line",);
    }

    fn round_trips(range: RangeInclusive<f32>, eased: bool) {
        let mut value = 0.0;
        let bar = ValueBar::new(&mut value, range, "test").eased(eased);
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            let v = bar.value_at(t);
            assert!((bar.to_t(v) - t).abs() < 1e-4, "t {t} -> value {v} -> t {}", bar.to_t(v));
        }
    }

    #[test]
    fn linear_positions_round_trip() {
        round_trips(-600.0..=600.0, false);
    }

    #[test]
    fn eased_positions_round_trip() {
        // min > 0 exercises the geometric branch; min == 0 the cubic one.
        round_trips(0.001..=49.999, true);
        round_trips(0.0..=100.0, true);
    }

    /// The magnet holds the bar at its target until an edit pulls clear of
    /// the window, and hands back an escaped value untouched. Both halves
    /// matter to the caller: inside the window the value it gets back is the
    /// target, which is what makes "did this edit escape" the same question
    /// as "did the value change".
    #[test]
    fn a_magnet_holds_the_bar_until_an_edit_pulls_clear() {
        let mut value = 400.0;
        let bar = ValueBar::new(&mut value, 380.0..=420.0, "Major third (¢)").magnet(400.0, 5.0);
        for v in [400.0f32, 396.0, 404.5, 395.0, 405.0] {
            assert_eq!(bar.snapped(v), 400.0, "{v} is inside the window");
        }
        for v in [394.9f32, 405.1, 380.0, 420.0] {
            assert_eq!(bar.snapped(v), v, "{v} is past the window");
        }
        let mut value = 400.0;
        let plain = ValueBar::new(&mut value, 380.0..=420.0, "Major third (¢)");
        assert_eq!(plain.snapped(396.0), 396.0, "no magnet, no pull");
    }

    #[test]
    fn integer_bars_snap() {
        let mut value = 0.0;
        let bar = ValueBar::new(&mut value, 1.0..=8.0, "test").integer();
        let v = bar.value_at(0.37);
        assert_eq!(v, v.round());
    }

    /// A bar's two text runs keep out of each other's way and stay inside the
    /// track, at every width.
    ///
    /// Three separate things in the paint have to hold for that, and nothing
    /// else in the suite relates one run to the other or either to the track:
    ///
    /// - the name's budget subtracts the room the readout needs, or the name
    ///   runs over the number (measured: 6pt of overlap at 160, 16pt at 120);
    /// - the name is held to ONE row, or it wraps to two and spills above and
    ///   below into the bars either side (a 29pt galley in a 20pt track);
    /// - both runs are offset by half their own height, or they sit a half-line
    ///   low with 7pt of a 17pt line below the track.
    ///
    /// Each is a live regression rather than a hypothetical: all three are
    /// clippy-clean and leave the rest of the suite green.
    /// `LayoutJob::simple_singleline` invites the second in particular — the
    /// name says the row cap is already set, and it is not.
    #[test]
    fn a_bars_name_and_readout_never_collide_or_leave_the_track() {
        let cases: [(&str, f32, RangeInclusive<f32>); 3] = [
            ("Harmonic seventh (¢)", 1000.0, SEVENTH_RANGE),
            ("Perfect fifth (¢)", 701.96, 680.0..=720.0),
            ("Sevenths angle", 45.0, 0.0..=90.0),
        ];
        for width in [400.0f32, 240.0, 180.0, 157.0, 120.0] {
            for (label, value, range) in cases.clone() {
                let mut value = value;
                let out = painted(width, |ui| {
                    ValueBar::new(&mut value, range.clone(), label).show(ui);
                });
                let mut runs: Vec<egui::Rect> = out
                    .iter()
                    .filter_map(|cs| match &cs.shape {
                        egui::Shape::Text(t) => Some(t.visual_bounding_rect()),
                        _ => None,
                    })
                    .collect();
                runs.sort_by(|a, b| a.left().total_cmp(&b.left()));
                assert_eq!(runs.len(), 2, "{label} at {width}pt painted {} runs", runs.len());
                let track = out
                    .iter()
                    .find_map(|cs| match &cs.shape {
                        egui::Shape::Rect(r) if r.fill == crate::theme::well() => Some(r.rect),
                        _ => None,
                    })
                    .expect("the bar painted no track");
                assert!(
                    runs[0].right() <= runs[1].left() + 0.5,
                    "{label} at {width}pt: the name reaches {} and the readout starts at {}",
                    runs[0].right(),
                    runs[1].left()
                );
                for run in &runs {
                    assert!(
                        run.top() >= track.top() - 0.5 && run.bottom() <= track.bottom() + 0.5,
                        "{label} at {width}pt: a run spans y {}..{} in a track of {}..{}",
                        run.top(),
                        run.bottom(),
                        track.top(),
                        track.bottom()
                    );
                }
            }
        }
    }

    /// A badged bar still says what drives it when its name has to be elided.
    ///
    /// Elision eats the tail, so a badge spelled into the END of a name is the
    /// first thing to go — and worse than merely lost: the UNBADGED name is the
    /// shorter of the two and draws in full at the same width, so the badged
    /// bar is the one that looks like it ran out of room. State has to sit
    /// where elision cannot reach it, which is the front.
    ///
    /// 157pt is the bar a 173pt column gives, and 173 is where the column
    /// floors — one separator drag from the default window, no resize needed.
    #[test]
    fn a_badged_bar_says_so_even_when_its_name_is_elided() {
        for width in [157.0f32, 180.0, 200.0, 400.0] {
            let mut value = 386.31;
            let out = painted(width, |ui| {
                ValueBar::new(&mut value, 380.0..=420.0, "Major third (¢)")
                    .badge("Meantone")
                    .show(ui);
            });
            let name = out
                .iter()
                .filter_map(|cs| match &cs.shape {
                    egui::Shape::Text(t) => Some((t.pos.x, painted_text(&t.galley))),
                    _ => None,
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .expect("the bar painted no text")
                .1;
            assert!(
                name.to_lowercase().contains("meantone"),
                "a {width}pt badged bar painted its name as {name:?}, which does not say so"
            );
        }
    }

    /// The name painted by a bar of `width` holding `value`, as its rendered
    /// (post-elision) width. The name is the left-hand run; the readout is
    /// right-aligned, so smallest x picks the name out.
    fn painted_name_width(width: f32, value: f32) -> f32 {
        let mut value = value;
        let out = painted(width, |ui| {
            ValueBar::new(&mut value, SEVENTH_RANGE, "Harmonic seventh (¢)").show(ui);
        });
        out.iter()
            .filter_map(|cs| match &cs.shape {
                egui::Shape::Text(t) => Some((t.pos.x, t.galley.size().x)),
                _ => None,
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .expect("the bar painted no text")
            .1
    }

    /// The harmonic-seventh bar's range, which straddles a digit boundary: the
    /// 12-TET value (and the param's default) is 1000.00, the just one 968.83.
    const SEVENTH_RANGE: RangeInclusive<f32> = 928.83..=1008.83;

    /// How wide a track the sweep readings below are taken across.
    const SWEEP_WIDTH: f32 = 240.0;

    /// Paint a progress bar at `time` and answer the fill's own rect and the
    /// run of it the sweep covered, or `None` where nothing swept.
    ///
    /// The colour is what identifies each: the accent fill and the shade above
    /// it are drawn nowhere else on this bar.
    fn swept(time: f64, fraction: Option<f32>) -> (Option<egui::Rect>, Option<egui::Rect>) {
        let shapes = shapes_at(SWEEP_WIDTH, time, |ui| {
            progress_bar(ui, fraction, "Rendering", "1200/5400");
        });
        let of = |want: egui::Color32| {
            let found: Vec<_> = filled_rects(&shapes)
                .into_iter()
                .filter(|(_, fill)| *fill == want)
                .map(|(r, _)| r)
                .collect();
            assert!(found.len() <= 1, "the bar drew {} rects in {want:?}", found.len());
            found.into_iter().next()
        };
        (of(theme::accent_fill()), of(theme::accent_fill_hover()))
    }

    /// The sweep is the fill's, and never touches the rest of the track.
    ///
    /// Past the frontier is what has NOT happened, and the bar's whole claim
    /// about that stretch is that nothing is there — so an animation reaching
    /// into it, at any strength, is drawing on the half of the bar that means
    /// "not yet". Swept across a period because the run is CLAMPED to the fill
    /// at both ends, and a clamp is exactly what holds at the phases either
    /// side of the one a fixture happens to pick.
    #[test]
    fn the_sweep_never_leaves_the_filled_part() {
        for step in 0..12 {
            let time = SWEEP_PERIOD * f64::from(step) / 12.0;
            for fraction in [0.05f32, 0.42, 1.0] {
                let (fill, run) = swept(time, Some(fraction));
                let fill = fill.expect("a bar with a fraction fills part of its track");
                let Some(run) = run else { continue };
                assert!(
                    fill.contains_rect(run),
                    "at {time}s a bar {fraction} full swept {run:?}, outside its {fill:?} fill",
                );
            }
        }
    }

    /// A bar with no fill sweeps nothing.
    ///
    /// The corollary of the rule above, and worth its own claim because it is
    /// the state a render opens in: no total reported yet, so the track draws
    /// empty by design. There is no filled part, so there is nothing to cross,
    /// and a sweep that fell back to the whole track here would be animating
    /// the one bar that has no reading in it at all.
    #[test]
    fn a_bar_with_no_fill_sweeps_nothing() {
        for step in 0..12 {
            let time = SWEEP_PERIOD * f64::from(step) / 12.0;
            for fraction in [None, Some(0.0)] {
                let (_, run) = swept(time, fraction);
                assert!(run.is_none(), "at {time}s a bar at {fraction:?} swept {run:?}");
            }
        }
    }

    /// The sweep is drawn from the CLOCK, at the same share of every fill.
    ///
    /// It says the bar is running, which is a different claim from how far it
    /// has got and must not be readable as that one. Taking a share of the fill
    /// is what makes it the same picture at every fraction — a sweep parked at
    /// the frontier, or one whose rate the fraction set, would be a second
    /// reading of the same number, and would contradict the first the moment it
    /// stalled, which is when this is the only thing left moving.
    #[test]
    fn the_sweep_sits_at_the_same_share_of_every_fill() {
        for step in 1..12 {
            let time = SWEEP_PERIOD * f64::from(step) / 12.0;
            let share = |fraction: f32| {
                let (fill, run) = swept(time, Some(fraction));
                let (fill, run) = (fill.unwrap(), run.unwrap());
                (
                    (run.left() - fill.left()) / fill.width(),
                    (run.right() - fill.left()) / fill.width(),
                )
            };
            let half = share(0.5);
            for fraction in [0.25f32, 1.0] {
                let other = share(fraction);
                assert!(
                    (other.0 - half.0).abs() < 0.01 && (other.1 - half.1).abs() < 0.01,
                    "at {time}s the sweep covers {other:?} of a bar {fraction} full and {half:?} \
                     of a half-full one",
                );
            }
        }
    }

    /// One period takes the sweep the whole way across the fill, once.
    ///
    /// Both halves are the claim. It reaches BOTH ends, so no part of the fill
    /// is permanently untouched and the sweep cannot be mistaken for a mark
    /// parked somewhere; and it only ever moves FORWARD, so the bar reads in
    /// the direction the work goes rather than rocking on the spot.
    ///
    /// Neither edge retreating, with one of them always advancing, is what
    /// "forward" has to mean here rather than one edge climbing: each edge is
    /// held at an end of the fill while the other crosses it — the sweep coming
    /// on at the start and going off at the frontier — so both stand still for
    /// part of a period, and a claim on either alone fails on the stretch the
    /// other one is moving.
    #[test]
    fn the_sweep_crosses_the_filled_part_once_a_period() {
        let mut last = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        let mut touched = (false, false);
        for step in 0..=12 {
            let time = SWEEP_PERIOD * f64::from(step) / 12.0;
            let (fill, run) = swept(time, Some(0.5));
            let fill = fill.unwrap();
            let Some(run) = run else { continue };
            let now = (run.left(), run.right());
            assert!(
                now.0 >= last.0 - 0.01 && now.1 >= last.1 - 0.01,
                "the sweep went backwards at {time}s, from {last:?} to {now:?}",
            );
            assert!(
                now.0 > last.0 + 0.01 || now.1 > last.1 + 0.01,
                "the sweep stood still at {time}s, at {now:?}",
            );
            last = now;
            touched.0 |= run.left() <= fill.left() + 0.01;
            touched.1 |= run.right() >= fill.right() - 0.01;
        }
        assert!(touched.0, "the sweep never reaches the start of the fill");
        assert!(touched.1, "the sweep never reaches the frontier");
    }

    /// The sweep is FLAT — one opaque colour, and no gradient anywhere.
    ///
    /// The house rule for a settings pane, and the reason this is a rect and
    /// not a soft band: every well, fill, button and handle here is one opaque
    /// colour, and the only gradients in the panel are the two bands whose
    /// value IS a ramp of colour. A mesh is what a gradient is built from in
    /// this crate, so a bar emitting one has grown a shading that means nothing.
    #[test]
    fn the_sweep_is_one_flat_colour() {
        let shapes = shapes_at(SWEEP_WIDTH, SWEEP_PERIOD * 0.5, |ui| {
            progress_bar(ui, Some(0.5), "Rendering", "1200/5400");
        });
        assert!(
            !shapes.iter().any(|s| matches!(s, egui::Shape::Mesh(_))),
            "the bar drew a mesh, which in a settings pane is a gradient",
        );
        let run = swept(SWEEP_PERIOD * 0.5, Some(0.5)).1.expect("the bar sweeps");
        assert!(run.width() > 0.0, "the sweep covers {run:?}");
    }

    /// The sweep is painted under everything the bar SAYS.
    ///
    /// Painted after the two text runs it would restate the name and the frame
    /// counts a shade lighter every time it passed them, which is the readings
    /// flickering — and they are what the bar is for.
    #[test]
    fn the_sweep_is_painted_under_the_bars_readings() {
        let shapes = shapes_at(SWEEP_WIDTH, SWEEP_PERIOD * 0.5, |ui| {
            progress_bar(ui, Some(0.5), "Rendering", "1200/5400");
        });
        let swept = shapes
            .iter()
            .position(|s| match s {
                egui::Shape::Rect(r) => r.fill == theme::accent_fill_hover(),
                _ => false,
            })
            .expect("the sweep is painted");
        let text = shapes
            .iter()
            .position(|s| matches!(s, egui::Shape::Text(_)))
            .expect("the bar draws its name");
        assert!(swept < text, "the sweep is painted over the bar's own readings");
    }

    /// A bar's NAME holds still while its number changes width.
    ///
    /// The name is elided against the room the readout leaves it, so a budget
    /// measured from the CURRENT readout re-elides the name the moment the
    /// value gains a digit — the name reflowing under the pointer mid-drag,
    /// which is the very thing the monospace readout buys for the digits. The
    /// budget has to come from the widest readout the bar's RANGE can produce.
    ///
    /// Swept across the band where it bites. Iosevka is 6pt per glyph and epaint
    /// rounds `wrap.max_width` only to whole points, so a digit is twelve times
    /// the rounding granularity and there is nothing to absorb it.
    #[test]
    fn a_bars_name_holds_still_while_its_number_changes_width() {
        for width in [174.0f32, 180.0, 184.0, 187.0, 200.0, 260.0] {
            let narrow = painted_name_width(width, 999.99);
            let wide = painted_name_width(width, 1000.00);
            assert!(
                (narrow - wide).abs() < 0.01,
                "a {width}pt bar draws its name {narrow}pt wide at 999.99 and {wide}pt at \
                 1000.00 — the name re-elides when the number gains a digit"
            );
        }
    }
}
