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

/// The preview line in `shapes`, as the points it was drawn through.
///
/// Shared by the two places that ask: the bar's own tests, which check WHERE
/// the line is drawn, and the Note section's, which checks WHICH curve it is.
/// The color is what identifies it — nothing else in a settings pane draws an
/// open path in it.
#[cfg(test)]
pub(crate) fn curve_points(shapes: &[egui::Shape]) -> Vec<egui::Pos2> {
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
        .flatten()
        .collect()
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
        if self.integer { v.round() } else { v }
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
        let centered = |galley: &egui::Galley, x: f32| {
            egui::pos2(x, rect.center().y - galley.size().y * 0.5)
        };
        painter.galley(centered(&label, rect.left() + text_pad), label, text_color);
        painter.galley(
            centered(&value, rect.right() - text_pad - value.size().x),
            value,
            theme::text(),
        );

        response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
    }
}

/// A read-only bar reporting how far something running in the background has
/// got: `fraction` of the track filled, `label` on the left and `value` read
/// out on the right.
///
/// The same shape as [`ValueBar`] — same track, fill, and text placement — so
/// a pane reads as one kind of control whether the number is one you set or
/// one you are being told. It senses hover only: there is nothing here to
/// drag, and a bar that moved under the pointer would claim there was.
///
/// `fraction` is `None` while the total is still unknown, and then the track
/// draws EMPTY rather than at zero — "no idea yet" and "none of it done" are
/// different things, and only one of them is a number.
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

    let radius = CornerRadius::same(bar_radius(scale));
    let painter = ui.painter();
    painter.rect_filled(rect, radius, theme::well());
    if let Some(t) = fraction {
        let mut fill = rect;
        fill.set_width(rect.width() * t.clamp(0.0, 1.0));
        painter.rect_filled(fill, radius, theme::accent_fill());
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
    use crate::widgets::probe::{painted, painted_text, shapes};

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
        assert!(
            curve_points(&shapes).is_empty(),
            "a plain bar painted a preview line",
        );
    }

    fn round_trips(range: RangeInclusive<f32>, eased: bool) {
        let mut value = 0.0;
        let bar = ValueBar::new(&mut value, range, "test").eased(eased);
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            let v = bar.value_at(t);
            assert!(
                (bar.to_t(v) - t).abs() < 1e-4,
                "t {t} -> value {v} -> t {}",
                bar.to_t(v)
            );
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
