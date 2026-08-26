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

// ---- The stripes that cross a running bar ----------------------------------
// Diagonal bands travelling along the bar, each drawn one rung above whatever
// it stands on: the fill's own raised shade on the near side of the frontier,
// the panel's faint-striping grey on the far side. The bar saying it is
// running, in the flattest thing that can lean — a polygon of one opaque
// colour off the skin.
//
// Flat is the constraint, not an economy. Nothing else in a settings pane is
// drawn in a gradient: the wells, the fills, the buttons and the handles are
// each one opaque colour, and the two bands that are not (a
// [`SpectrumBar`](super::gradient::SpectrumBar) track, a
// [`GradientPreview`](super::gradient::GradientPreview)) are gradients because
// the value they draw IS a ramp of colour. A soft-edged band here would be the
// one shading in the pane that means nothing.
//
// The pattern crosses the WHOLE track and changes colour at the frontier
// rather than stopping at it, which is what keeps the two claims apart. How far
// the work has got is the frontier alone, and a stripe cannot be misread as a
// second helping of it, because a stripe is the same picture wherever it sits
// and only the frontier ever reads as a quantity. That it is running at all is
// the motion, and the far side is where that has to be legible: the state a
// render opens in has no total yet, so it has no fill at all.

/// Distance along the bar from one stripe to the next.
///
/// Sized against the row it lives in ([`theme::ROW_HEIGHT`]) rather than
/// against the bar's length, which the pane's width sets: a stripe is read
/// against the thickness it crosses, so a pitch near two thirds of the height
/// is the same rhythm in a narrow column and a wide one.
///
/// The design size, at [chrome scale](theme::ui_scale) 1.0.
const STRIPE_PITCH: f32 = 13.0;

/// How much of a pitch the stripe covers, the rest being the ground it stands
/// on.
///
/// Even, so neither the stripe nor the gap between two reads as the figure and
/// the other as the ground — the pattern is meant to be a texture that moves,
/// and a thin stripe on a wide ground is a row of marks that could be counted.
const STRIPE_DUTY: f32 = 0.5;

/// How many stripes pass a fixed point on the bar each second.
///
/// Paced against what the bar is measuring: a render is minutes of work, so it
/// has to say "running" to a glance without asking to be watched. Two glances a
/// second apart find the pattern somewhere else, and nothing about it reads as
/// a countdown — which a rate several times this would, the eye taking a beat
/// that fast as one tick per unit of something.
///
/// A rate in stripes rather than a speed in points, so the pitch above is the
/// only thing that sets the rhythm: a speed would have to be retuned every time
/// the pitch moved to keep the same one.
const STRIPE_RATE: f32 = 1.2;

/// How far a stripe leans, in points along the bar per point down it. 1.0 is
/// the 45° of the pattern this is: the lean is what makes a band read as
/// travelling ALONG the bar rather than as a shutter opening and closing.
const STRIPE_LEAN: f32 = 1.0;

/// Area under which a cut stripe is dropped rather than painted — a sliver off
/// an end of the track, too thin to put a pixel down and still a shape to
/// tessellate.
const STRIPE_MIN_AREA: f32 = 0.1;

/// How many segments each corner of a [`rounded_outline`] is drawn in.
///
/// The outline is a stripe's cutter rather than anything painted, so it only
/// has to be closer to the arc than a viewer can see it miss: four segments
/// leave the shared control radius by under a fifth of a point at the largest
/// [chrome scale](theme::ui_scale) on offer.
const OUTLINE_STEPS: usize = 4;

/// The outline of a rounded rect, clockwise, as a convex polygon a stripe can
/// be cut against.
///
/// The radius is clamped the way epaint clamps a rect shape's — to half the
/// shorter side — so the cutter and the fill it is cutting to agree about the
/// shape of a nearly-empty bar, where the fill is narrower than two corners.
fn rounded_outline(rect: egui::Rect, radius: f32) -> Vec<egui::Pos2> {
    let radius = radius.clamp(0.0, rect.width().min(rect.height()) * 0.5);
    // Each corner as the centre it turns about and the direction it starts
    // from, in the screen's own angles: x to the right, y DOWN, so the quarter
    // turns run clockwise on screen from the top-left corner.
    let corners = [
        (egui::pos2(rect.left() + radius, rect.top() + radius), 180.0_f32),
        (egui::pos2(rect.right() - radius, rect.top() + radius), 270.0),
        (egui::pos2(rect.right() - radius, rect.bottom() - radius), 0.0),
        (egui::pos2(rect.left() + radius, rect.bottom() - radius), 90.0),
    ];
    let mut outline = Vec::with_capacity(corners.len() * (OUTLINE_STEPS + 1));
    for (centre, from) in corners {
        for step in 0..=OUTLINE_STEPS {
            let angle = (from + 90.0 * step as f32 / OUTLINE_STEPS as f32).to_radians();
            outline.push(centre + egui::vec2(angle.cos(), angle.sin()) * radius);
        }
    }
    outline
}

/// `poly` cut down to the part of it inside `region` — both convex, both wound
/// clockwise.
///
/// Sutherland–Hodgman: cut against one of the region's edges at a time. A
/// convex polygon cut by a half-plane is still convex and still wound the same
/// way, so what comes out is something [`egui::Shape::convex_polygon`] can take
/// and nothing here has to triangulate.
fn clipped(poly: &[egui::Pos2], region: &[egui::Pos2]) -> Vec<egui::Pos2> {
    let mut poly = poly.to_vec();
    for edge in 0..region.len() {
        if poly.is_empty() {
            return poly;
        }
        let (from, to) = (region[edge], region[(edge + 1) % region.len()]);
        // Positive to the RIGHT of the edge's direction, which is the inside of
        // a clockwise outline once y points down.
        let side =
            |p: egui::Pos2| (to.x - from.x) * (p.y - from.y) - (to.y - from.y) * (p.x - from.x);
        let cut = std::mem::take(&mut poly);
        for i in 0..cut.len() {
            let (p, q) = (cut[i], cut[(i + 1) % cut.len()]);
            let (sp, sq) = (side(p), side(q));
            let crossing = || p + (q - p) * (sp / (sp - sq));
            match (sp >= 0.0, sq >= 0.0) {
                (true, true) => poly.push(q),
                (true, false) => poly.push(crossing()),
                (false, true) => poly.extend([crossing(), q]),
                (false, false) => {}
            }
        }
    }
    poly
}

/// The area `poly` encloses, positive for the clockwise winding everything
/// here is wound in.
fn area(poly: &[egui::Pos2]) -> f32 {
    let shoelace: f32 = (0..poly.len())
        .map(|i| {
            let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
            p.x * q.y - q.x * p.y
        })
        .sum();
    shoelace * 0.5
}

/// Paint the travelling stripes over `region` in `colour`.
///
/// The pattern is laid out across `bar` and only shown inside `region`, which
/// is how one pattern comes out in two colours: the same bands are painted
/// twice, cut to the whole track and then to the fill, with the fill's own
/// opaque rect between the two passes.
///
/// `travel` is how far along the bar the pattern stands this frame, in points,
/// and `corner` the radius `region` is drawn with — the cut follows it, so a
/// band that runs off an end wears that end's curve instead of poking a square
/// corner out of it.
fn stripes(
    painter: &egui::Painter,
    bar: egui::Rect,
    region: egui::Rect,
    corner: u8,
    pitch: f32,
    travel: f32,
    colour: Color32,
) {
    if region.width() <= 0.0 || region.height() <= 0.0 || pitch <= 0.0 {
        return;
    }
    let outline = rounded_outline(region, f32::from(corner));
    // How far a band shifts between the top of the bar and the bottom, and so
    // how much further along the bar the pattern reaches at the bottom edge
    // than the top one.
    let lean = STRIPE_LEAN * bar.height();
    let band = |along: f32| {
        [
            egui::pos2(along, bar.top()),
            egui::pos2(along + pitch * STRIPE_DUTY, bar.top()),
            egui::pos2(along + pitch * STRIPE_DUTY - lean, bar.bottom()),
            egui::pos2(along - lean, bar.bottom()),
        ]
    };
    // Every band whose reach touches the region: the pattern is anchored to the
    // bar's left end, and the lean is what carries the last one further along at
    // the bottom edge than at the top.
    let first = ((region.left() - travel) / pitch).floor();
    let bands = ((region.width() + lean) / pitch).ceil().max(0.0) as usize;
    for step in 0..=bands {
        let poly = clipped(&band(travel + (first + step as f32) * pitch), &outline);
        if area(&poly) > STRIPE_MIN_AREA {
            painter.add(egui::Shape::convex_polygon(poly, colour, egui::Stroke::NONE));
        }
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
/// **Diagonal stripes travel along it while it stands**, on the clock rather
/// than on the value, so the bar says it is running as well as how far along it
/// is. Those are two jobs and only the second is a number: a bar can go a long
/// while without its fill visibly moving — a render counting to five thousand
/// frames advances the edge by a twentieth of a point a frame — and a still bar
/// and a hung one look alike.
///
/// The stripes cross the whole track, taking the shade above whichever side of
/// the frontier they stand on. See [`stripes`] for why they do not stop at it.
///
/// `fraction` is `None` while the total is still unknown, and then the track
/// draws EMPTY rather than at zero — "no idea yet" and "none of it done" are
/// different things, and only one of them is a number. That state is the one
/// with the least to show and the most to say, since it is where a render
/// opens: nothing about the frontier is a reading yet, and the stripes are the
/// whole of what the bar can honestly claim.
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

    // Where the pattern stands this frame, from the CLOCK alone: two of these
    // bars on screen stripe together rather than each from whenever it
    // appeared. Wrapped to one pitch before it is a length, so the arithmetic
    // is exact through a render that runs for an hour rather than losing points
    // off an f32 that has grown into the thousands.
    let pitch = STRIPE_PITCH * scale;
    let travel =
        (ui.ctx().input(|i| i.time) * f64::from(STRIPE_RATE)).rem_euclid(1.0) as f32 * pitch;
    // Unconditional, unlike a bar whose animation is its fill's: this one is
    // only on screen while a render is running, and the stripes cross the whole
    // track whether or not the fill has reached them.
    ui.ctx().request_repaint();

    let painter = ui.painter();
    painter.rect_filled(rect, radius, theme::well());
    // The rung a raised surface sits at, which is what the skin keeps this grey
    // for. On the dark side of the frontier it is the only thing in the well.
    stripes(painter, rect, rect, corner, pitch, travel, theme::surface_faint());
    if let Some(fill) = fill {
        // Opaque over the pass above, so the pattern CHANGES colour at the
        // frontier rather than showing two of itself through one. The stripes
        // under the fill are painted and covered rather than skipped: what
        // covers them is a rounded rect, and cutting them to it would mean
        // cutting to the outside of a curve, which is not one half-plane.
        painter.rect_filled(fill, radius, theme::accent_fill());
        // The fill a shade up: a colour the pane already wears rather than a
        // new one, so a stripe cannot read as a state the bar has entered.
        // Opaque, like every accent mix in the skin — an alpha over the fill
        // would be a third colour that exists nowhere else in the panel.
        stripes(painter, rect, fill, corner, pitch, travel, theme::accent_fill_hover());
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
    use crate::widgets::probe::{
        filled_polys, filled_rects, painted, painted_text, shapes, shapes_at,
    };

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

    /// How wide a track the stripe readings below are taken across.
    const STRIPED_WIDTH: f32 = 240.0;

    /// One turn of the pattern: the time a stripe takes to reach where the one
    /// before it stood.
    const STRIPE_TURN: f64 = 1.0 / STRIPE_RATE as f64;

    /// One stripe as it was painted: the points it was drawn through, and the
    /// shade that says which side of the frontier it stands on.
    type Stripe = (Vec<egui::Pos2>, Color32);

    /// Paint a progress bar at `time` and answer its track, its fill, and every
    /// polygon on it in paint order.
    ///
    /// A polygon is a stripe and nothing else here: the rest of the bar is
    /// rects and text, so anything that came through as a path leans.
    fn striped(time: f64, fraction: Option<f32>) -> (egui::Rect, Option<egui::Rect>, Vec<Stripe>) {
        let shapes = shapes_at(STRIPED_WIDTH, time, |ui| {
            progress_bar(ui, fraction, "Rendering", "1200/5400");
        });
        let rect = |want: Color32| {
            filled_rects(&shapes).into_iter().find(|(_, fill)| *fill == want).map(|(r, _)| r)
        };
        (
            rect(theme::well()).expect("the bar draws a track"),
            rect(theme::accent_fill()),
            filled_polys(&shapes),
        )
    }

    /// Where `poly` reaches at height `y`, or `None` where it does not reach
    /// that deep.
    ///
    /// A convex polygon meets a horizontal line in one run, so this is a
    /// stripe's width measured at a named depth — and a lean is read by
    /// measuring the same stripe at two of them.
    fn span_at(poly: &[egui::Pos2], y: f32) -> Option<(f32, f32)> {
        let mut hits = Vec::new();
        for i in 0..poly.len() {
            let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
            if (p.y <= y) != (q.y <= y) {
                hits.push(p.x + (q.x - p.x) * (y - p.y) / (q.y - p.y));
            }
        }
        let lo = hits.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = hits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        (hits.len() >= 2).then_some((lo, hi))
    }

    /// Whether `p` is inside `rect` rounded off by `radius`, which is the shape
    /// the bar actually draws — the plain rect would pass a point standing in
    /// the square corner the rounding cuts away.
    fn inside_rounded(rect: egui::Rect, radius: f32, p: egui::Pos2) -> bool {
        let radius = radius.clamp(0.0, rect.width().min(rect.height()) * 0.5);
        let square = egui::Rect::from_min_max(
            egui::pos2(rect.left() + radius, rect.top() + radius),
            egui::pos2(rect.right() - radius, rect.bottom() - radius),
        );
        p.distance(square.clamp(p)) <= radius + 0.01
    }

    /// Every stripe's left edge, at the depth the track is widest.
    ///
    /// Mid-height on purpose: the rounding bites only within a corner radius of
    /// the top and bottom, so a reading taken there is of the pattern and not
    /// of the curve it is cut to.
    fn starts_at_mid(track: egui::Rect, polys: &[Stripe]) -> Vec<f32> {
        polys.iter().filter_map(|(poly, _)| span_at(poly, track.center().y)).map(|s| s.0).collect()
    }

    /// The stripes lean, all by the same amount.
    ///
    /// The lean is the whole reason this reads as travelling rather than as a
    /// shutter: an upright band crossing a bar is as much a closing door as an
    /// advancing one, and the eye takes a leaning one as going the way it
    /// points. Measured on stripes that cross the track whole, since a shear is
    /// horizontal and so leaves a stripe's width at any depth alone — a
    /// measurement narrower than that is one an end of the track has cut.
    #[test]
    fn the_stripes_lean() {
        let (track, _, polys) = striped(0.3, None);
        let (near_top, near_bottom) = (track.top() + 1.0, track.bottom() - 1.0);
        let mut whole = 0;
        for (poly, _) in &polys {
            let (Some(top), Some(bottom)) = (span_at(poly, near_top), span_at(poly, near_bottom))
            else {
                continue;
            };
            let width = STRIPE_PITCH * STRIPE_DUTY;
            if (top.1 - top.0 - width).abs() > 0.01 || (bottom.1 - bottom.0 - width).abs() > 0.01 {
                continue;
            }
            whole += 1;
            let lean = (top.0 - bottom.0) / (near_bottom - near_top);
            assert!(
                (lean - STRIPE_LEAN).abs() < 0.01,
                "a stripe leans {lean} points along the bar per point down it, not {STRIPE_LEAN}",
            );
        }
        assert!(whole > 4, "only {whole} stripes crossed the track whole to be measured");
    }

    /// A stripe is FLAT — one opaque colour, and no gradient anywhere.
    ///
    /// The house rule for a settings pane, and the reason a stripe is a polygon
    /// and not a soft band: every well, fill, button and handle here is one
    /// opaque colour, and the only gradients in the panel are the two bands
    /// whose value IS a ramp of colour. A mesh is what a gradient is built from
    /// in this crate, so a bar emitting one has grown a shading that means
    /// nothing.
    #[test]
    fn a_stripe_is_one_flat_colour() {
        let shapes = shapes_at(STRIPED_WIDTH, 0.3, |ui| {
            progress_bar(ui, Some(0.5), "Rendering", "1200/5400");
        });
        assert!(
            !shapes.iter().any(|s| matches!(s, egui::Shape::Mesh(_))),
            "the bar drew a mesh, which in a settings pane is a gradient",
        );
        let polys = filled_polys(&shapes);
        assert!(!polys.is_empty(), "the bar drew no stripes to check");
        for (_, colour) in &polys {
            assert!(
                *colour == theme::surface_faint() || *colour == theme::accent_fill_hover(),
                "a stripe is {colour:?}, neither shade the bar's two sides wear",
            );
        }
    }

    /// No stripe leaves the track, at any phase or any fill.
    ///
    /// A band laid across the bar reaches past both ends of it, so what keeps
    /// it inside is the cut — and the shape it is cut to is the ROUNDED track,
    /// not the rect. Cutting to the rect passes every reading but the one that
    /// matters: the corner, where a square end of a band would stand outside
    /// the curve the rest of the pane is drawn to.
    #[test]
    fn no_stripe_leaves_the_track() {
        for step in 0..6 {
            let time = STRIPE_TURN * f64::from(step) / 6.0;
            for fraction in [None, Some(0.03), Some(0.5), Some(1.0)] {
                let (track, _, polys) = striped(time, fraction);
                let radius = f32::from(bar_radius(1.0));
                for (poly, _) in polys.iter().filter(|(_, c)| *c == theme::surface_faint()) {
                    for p in poly {
                        assert!(
                            inside_rounded(track, radius, *p),
                            "at {time}s a stripe reaches {p:?}, outside the {track:?} track",
                        );
                    }
                }
            }
        }
    }

    /// A stripe takes the shade of the side of the frontier it stands on, and
    /// is cut to that side.
    ///
    /// The pattern says the bar is running and the frontier says how far it has
    /// got, and the second is the reading: a stripe carrying the fill's shade
    /// out past the frontier would put the fill's own colour on the part of the
    /// bar whose whole meaning is that nothing has happened there yet.
    ///
    /// Cut to the fill's own rounded shape rather than to the track's, which is
    /// what a nearly-empty bar is here to catch: a fill narrower than two
    /// corners is rounded to HALF ITS WIDTH, the clamp epaint puts on a rect
    /// shape, so a stripe cut to the track's radius would stand outside the
    /// fill it is meant to be lighting.
    #[test]
    fn a_stripe_takes_the_shade_of_the_side_it_stands_on() {
        for step in 0..6 {
            let time = STRIPE_TURN * f64::from(step) / 6.0;
            for fraction in [0.03f32, 0.5, 1.0] {
                let (_, fill, polys) = striped(time, Some(fraction));
                let fill = fill.expect("a bar with a fraction fills part of its track");
                let radius = f32::from(bar_radius(1.0));
                for (poly, _) in polys.iter().filter(|(_, c)| *c == theme::accent_fill_hover()) {
                    for p in poly {
                        assert!(
                            inside_rounded(fill, radius, *p),
                            "at {time}s a bar {fraction} full lights {p:?}, outside its \
                             {fill:?} fill",
                        );
                    }
                }
            }
        }
        let lit = striped(0.3, Some(0.5))
            .2
            .iter()
            .filter(|(_, c)| *c == theme::accent_fill_hover())
            .count();
        assert!(lit > 4, "only {lit} stripes wear the fill's shade on a half-full bar");
    }

    /// The track's stripes are painted UNDER the fill.
    ///
    /// One pattern in two colours rather than two patterns: the dark pass runs
    /// the whole track, and what makes the near side change shade is the fill's
    /// own opaque rect covering that pass before the light one is drawn. Paint
    /// the fill first and the dark stripes are on top of it — the far side's
    /// texture laid over the near side's colour.
    #[test]
    fn the_track_stripes_are_painted_under_the_fill() {
        let shapes = shapes_at(STRIPED_WIDTH, 0.3, |ui| {
            progress_bar(ui, Some(0.5), "Rendering", "1200/5400");
        });
        let at = |want: Color32| {
            let of = |s: &egui::Shape| match s {
                egui::Shape::Rect(r) => r.fill == want,
                egui::Shape::Path(p) => p.fill == want,
                _ => false,
            };
            let first = shapes.iter().position(of).expect("the bar paints it");
            let last = shapes.iter().rposition(of).expect("the bar paints it");
            (first, last)
        };
        let dark = at(theme::surface_faint());
        let fill = at(theme::accent_fill());
        let light = at(theme::accent_fill_hover());
        assert!(dark.1 < fill.0, "a track stripe is painted over the fill");
        assert!(fill.1 < light.0, "the fill is painted over its own stripes");
    }

    /// A bar with NO fill still stripes, and still moves.
    ///
    /// The state a render opens in: no total reported yet, so the track draws
    /// empty by design and the frontier has nothing to say. It is the bar with
    /// the least to show and the most to tell — a render that has not counted
    /// its first frame looks exactly like one that has hung — so it is the one
    /// state the pattern cannot be allowed to sit out.
    #[test]
    fn a_bar_with_no_fill_still_stripes() {
        for fraction in [None, Some(0.0)] {
            let (track, fill, polys) = striped(0.0, fraction);
            assert!(fill.is_none() || fill.is_some_and(|f| f.width() == 0.0));
            assert!(polys.len() > 4, "a bar at {fraction:?} drew {} stripes", polys.len());
            let moved = striped(STRIPE_TURN / 3.0, fraction).2;
            assert_ne!(
                starts_at_mid(track, &polys),
                starts_at_mid(track, &moved),
                "a bar at {fraction:?} drew the same stripes a third of a turn later",
            );
        }
    }

    /// The light stripes are the dark ones, re-coloured — not a second pattern.
    ///
    /// Two patterns would beat against each other: they would agree at the
    /// phase a fixture happened to pick and drift apart everywhere else, and
    /// what shows at the frontier is a stripe with a step in it. Read at
    /// mid-height, where the track is widest, so a stripe's left edge is the
    /// pattern's own and not something a corner cut.
    #[test]
    fn a_light_stripe_stands_where_a_dark_one_does() {
        for step in 0..6 {
            let time = STRIPE_TURN * f64::from(step) / 6.0;
            let (track, _, polys) = striped(time, Some(0.6));
            let side = |want: Color32| {
                let of: Vec<_> =
                    polys.iter().filter(|(_, c)| *c == want).cloned().collect::<Vec<_>>();
                starts_at_mid(track, &of)
            };
            let dark = side(theme::surface_faint());
            for start in side(theme::accent_fill_hover()) {
                assert!(
                    dark.iter().any(|d| (d - start).abs() < 0.01),
                    "at {time}s a light stripe starts at {start}, where no dark one does: {dark:?}",
                );
            }
        }
    }

    /// The pattern travels ALONG the bar, one stripe per turn.
    ///
    /// Both halves are the claim. It goes forward, so the bar reads in the
    /// direction the work does rather than rocking on the spot; and it covers
    /// exactly one pitch per turn, which is what makes the rate a rate in
    /// stripes — retune the pitch and the rhythm the eye reads is unchanged.
    ///
    /// Measured on the leading edges modulo the pitch, because a pattern that
    /// wraps has no single edge to follow: one leaves the far end while another
    /// arrives at the near one.
    #[test]
    fn the_pattern_travels_one_stripe_a_turn() {
        let phase = |time: f64| {
            let (track, _, polys) = striped(time, None);
            let starts = starts_at_mid(track, &polys);
            // The leftmost stripe starts where the TRACK does, its own start
            // cut away, so it says nothing about where the pattern stands.
            let uncut = starts
                .iter()
                .find(|s| **s > track.left() + 0.01)
                .expect("some stripe starts inside the track");
            (uncut - track.left()).rem_euclid(STRIPE_PITCH)
        };
        let steps = 8;
        for step in 0..steps {
            let (from, to) = (
                phase(STRIPE_TURN * f64::from(step) / f64::from(steps)),
                phase(STRIPE_TURN * f64::from(step + 1) / f64::from(steps)),
            );
            let by = (to - from).rem_euclid(STRIPE_PITCH);
            let want = STRIPE_PITCH / steps as f32;
            assert!(
                (by - want).abs() < 0.01,
                "an eighth of a turn moved the pattern {by} points along the bar, not {want}",
            );
        }
    }

    /// The stripes are painted under everything the bar SAYS.
    ///
    /// Painted after the two text runs they would restate the name and the
    /// frame counts a shade lighter every time one passed, which is the
    /// readings flickering — and they are what the bar is for.
    #[test]
    fn the_stripes_are_painted_under_the_bars_readings() {
        let shapes = shapes_at(STRIPED_WIDTH, 0.3, |ui| {
            progress_bar(ui, Some(0.5), "Rendering", "1200/5400");
        });
        let striped = shapes
            .iter()
            .rposition(|s| matches!(s, egui::Shape::Path(p) if p.fill != Color32::TRANSPARENT))
            .expect("the bar is striped");
        let text = shapes
            .iter()
            .position(|s| matches!(s, egui::Shape::Text(_)))
            .expect("the bar draws its name");
        assert!(striped < text, "a stripe is painted over the bar's own readings");
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
