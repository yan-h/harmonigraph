//! The Tape tab (#1719): what the tape holds along the song, on a strip that
//! plays the recording from wherever it is clicked while the transport is
//! stopped. Bitwig's own playhead cannot do this: a plugin is not told when
//! it is clicked while stopped.
//!
//! The strip shows the whole recording until it is zoomed (pinch or
//! Ctrl+wheel) or panned (drag, or a sideways scroll). The ordinary wheel is
//! left to the settings column's own scrolling.

use egui::{pos2, vec2, Align2, Rect, Sense, Stroke};

use crate::tape::{TapeRequest, TapeView};
use crate::theme;

/// The narrowest stretch of song the strip will zoom to, in seconds.
const MIN_SECONDS: f64 = 2.0;
/// The space a label needs, in points, before the ruler labels every bar or
/// every few.
const LABEL_SPACING: f32 = 44.0;

/// The stretch of song the strip shows. `zoomed` is set by any zoom or pan,
/// until Fit clears it; until then the strip follows the recording's length.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Window {
    from: f64,
    to: f64,
    zoomed: bool,
}

impl Window {
    /// The whole recording, the reported position and what sounds, with a
    /// little room past the end.
    fn fit(view: &TapeView) -> Window {
        let end = [view.end(), view.position, view.heard].into_iter().flatten().fold(0.0, f64::max);
        Window { from: 0.0, to: (end * 1.05).max(MIN_SECONDS * 4.0), zoomed: false }
    }

    fn seconds(&self) -> f64 {
        self.to - self.from
    }

    /// Zoom by `factor` (above 1 zooms in) about song seconds `at`.
    fn zoom(&mut self, factor: f64, at: f64) {
        let width = (self.seconds() / factor).max(MIN_SECONDS);
        let share = (at - self.from) / self.seconds();
        self.from = (at - share * width).max(0.0);
        self.to = self.from + width;
        self.zoomed = true;
    }

    fn pan(&mut self, seconds: f64) {
        let shift = seconds.max(-self.from);
        self.from += shift;
        self.to += shift;
        self.zoomed = true;
    }
}

/// Song seconds as `m:ss.s`.
fn clock(seconds: f64) -> String {
    let minutes = (seconds / 60.0).floor();
    format!("{minutes:.0}:{:04.1}", seconds - minutes * 60.0)
}

pub(super) fn tape_pane(ui: &mut egui::Ui, view: &mut TapeView) {
    if !view.present {
        ui.label("The tape records inside the plugin only.");
        return;
    }
    let id = ui.id().with("tape-window");
    let mut window = ui.data(|d| d.get_temp::<Window>(id)).unwrap_or_else(|| Window::fit(view));
    if !window.zoomed {
        window = Window::fit(view);
    }
    ui.horizontal_wrapped(|ui| {
        let stop = ui.add_enabled(view.heard.is_some(), egui::Button::new("Stop"));
        if stop.on_hover_text("End the audition.").clicked() {
            view.request = Some(TapeRequest::Stop);
        }
        let fit = ui.add_enabled(window.zoomed, egui::Button::new("Fit"));
        if fit.on_hover_text("Show the whole recording.").clicked() {
            window = Window::fit(view);
        }
        ui.label(format!("{} MiB", view.bytes >> 20))
            .on_hover_text("Memory the recording holds. It is never saved with the project.");
    });
    let hint = if !view.enabled {
        "No audition while the analyzer listens to the sidechain."
    } else if view.rolling {
        "Stop the transport to audition."
    } else if view.spans.is_empty() {
        "Nothing recorded yet. The song records as it plays."
    } else {
        "Click the strip to play the recording from there."
    };
    ui.label(egui::RichText::new(hint).color(theme::text_dim()));
    strip(ui, view, &mut window);
    ui.data_mut(|d| d.insert_temp(id, window));
}

fn strip(ui: &mut egui::Ui, view: &mut TapeView, window: &mut Window) {
    let row = theme::row_height(theme::ui_scale(ui.ctx()));
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), 3.0 * row), Sense::click_and_drag());
    let width = f64::from(rect.width().max(1.0));
    let seconds_at = |window: &Window, x: f32| {
        window.from + f64::from(x - rect.left()) / width * window.seconds()
    };

    if response.contains_pointer() {
        let (zoom, sideways) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta.x));
        if zoom != 1.0 {
            if let Some(pointer) = response.hover_pos() {
                let at = seconds_at(window, pointer.x);
                window.zoom(f64::from(zoom), at);
            }
        }
        if sideways != 0.0 {
            window.pan(-f64::from(sideways) / width * window.seconds());
        }
    }
    if response.dragged() {
        window.pan(-f64::from(response.drag_delta().x) / width * window.seconds());
    }
    if response.clicked() && view.enabled && !view.rolling {
        if let Some(pointer) = response.interact_pointer_pos() {
            view.request = Some(TapeRequest::Cue(seconds_at(window, pointer.x).max(0.0)));
        }
    }

    let window = *window;
    let x_of =
        |seconds: f64| rect.left() + ((seconds - window.from) / window.seconds() * width) as f32;
    let painter = ui.painter_at(rect);
    let radius = theme::control_radius(theme::ui_scale(ui.ctx()));
    painter.rect_filled(rect, radius, theme::well());
    let ruler = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + row));
    let lane = Rect::from_min_max(pos2(rect.left(), ruler.bottom()), rect.max);

    for span in &view.spans {
        let (left, right) = (x_of(span.start), x_of(span.end));
        if right < rect.left() || left > rect.right() {
            continue;
        }
        let fill = if span.warm {
            theme::accent_fill()
        } else {
            theme::accent_fill().gamma_multiply(0.45)
        };
        let body = Rect::from_min_max(
            pos2(left, lane.top() + 2.0),
            pos2(right.max(left + 1.0), lane.bottom() - 2.0),
        );
        painter.rect_filled(body, 0, fill);
    }
    draw_ruler(&painter, ui, view, &window, ruler, lane, &x_of);

    if let Some(position) = view.position {
        let x = x_of(position);
        painter.line_segment(
            [pos2(x, ruler.top()), pos2(x, rect.bottom())],
            Stroke::new(theme::LINE_WIDTH, theme::text_dim()),
        );
    }
    if let Some(heard) = view.heard {
        let x = x_of(heard);
        painter.line_segment(
            [pos2(x, lane.top()), pos2(x, rect.bottom())],
            Stroke::new(theme::LINE_WIDTH, theme::accent()),
        );
    }
    if let Some(pointer) = response.hover_pos() {
        let at = seconds_at(&window, pointer.x).max(0.0);
        painter.line_segment(
            [pos2(pointer.x, lane.top()), pos2(pointer.x, lane.bottom())],
            Stroke::new(theme::GRID_WIDTH, theme::text()),
        );
        response.on_hover_text_at_pointer(describe(view, at));
    }
}

fn draw_ruler(
    painter: &egui::Painter,
    ui: &egui::Ui,
    view: &TapeView,
    window: &Window,
    ruler: Rect,
    lane: Rect,
    x_of: &dyn Fn(f64) -> f32,
) {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let tick = Stroke::new(theme::GRID_WIDTH, theme::hairline());
    let label = |x: f32, text: String| {
        painter.text(
            pos2(x + 3.0, ruler.center().y),
            Align2::LEFT_CENTER,
            text,
            font.clone(),
            theme::text_dim(),
        );
    };
    let per_second = (x_of(1.0) - x_of(0.0)).max(f32::EPSILON);
    if let Some(bar) = view.bar_seconds(window.from) {
        let per_bar = (bar as f32 * per_second).max(f32::EPSILON);
        let every = [1, 2, 4, 8, 16, 32, 64, 128, 256, 512]
            .into_iter()
            .find(|n| per_bar * *n as f32 >= LABEL_SPACING)
            .unwrap_or(1024);
        for (seconds, bar) in view.bar_lines(window.from, window.to) {
            let x = x_of(seconds);
            let labelled = (bar - 1) % every == 0;
            if labelled || per_bar >= 4.0 {
                let top = if labelled { ruler.top() } else { lane.top() };
                painter.line_segment([pos2(x, top), pos2(x, lane.bottom())], tick);
            }
            if labelled {
                label(x, bar.to_string());
            }
        }
        return;
    }
    // No meter from the host: minutes and seconds.
    let every = [1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0]
        .into_iter()
        .find(|n| per_second * *n as f32 >= LABEL_SPACING * 1.5)
        .unwrap_or(1200.0);
    let mut seconds = (window.from / every).ceil() * every;
    while seconds < window.to {
        let x = x_of(seconds);
        painter.line_segment([pos2(x, ruler.top()), pos2(x, lane.bottom())], tick);
        label(x, clock(seconds).trim_end_matches(".0").to_owned());
        seconds += every;
    }
}

/// The hover text for song seconds `at`.
fn describe(view: &TapeView, at: f64) -> String {
    let place = match view.bar_at(at) {
        Some((bar, beat)) => format!("Bar {bar}.{beat} · {}", clock(at)),
        None => clock(at),
    };
    let held = match view.span_at(at) {
        Some(span) => {
            let ago = span.recorded.elapsed().as_secs_f64();
            let ago =
                if ago < 90.0 { format!("{ago:.0} s") } else { format!("{:.0} min", ago / 60.0) };
            format!("{}, recorded {ago} ago", if span.warm { "warm" } else { "cold" })
        }
        None => "nothing recorded".to_owned(),
    };
    format!("{place}\n{held}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::TapeSpan;

    /// One pass of the pane in a 400 by 300 point window; the strip's rect.
    fn pass(ctx: &egui::Context, view: &mut TapeView, events: Vec<egui::Event>) -> Option<Rect> {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let output = ctx.run_ui(input, |ui| tape_pane(ui, view));
        output.shapes.iter().find_map(|cs| match &cs.shape {
            egui::Shape::Rect(r) if r.fill == theme::well() => Some(r.rect),
            _ => None,
        })
    }

    fn click(at: egui::Pos2) -> Vec<egui::Event> {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        vec![egui::Event::PointerMoved(at), button(true), button(false)]
    }

    #[test]
    fn a_click_asks_for_the_song_position_under_it_unless_the_transport_rolls() {
        let ctx = egui::Context::default();
        // 40 s recorded: the strip fits 0 to 42 s.
        let span =
            TapeSpan { start: 0.0, end: 40.0, warm: true, recorded: std::time::Instant::now() };
        let mut view =
            TapeView { present: true, enabled: true, spans: vec![span], ..Default::default() };
        let strip = pass(&ctx, &mut view, vec![]).expect("the strip is drawn");
        let at = pos2(strip.left() + strip.width() * 0.25, strip.bottom() - 4.0);
        pass(&ctx, &mut view, click(at));
        match view.request.take() {
            Some(TapeRequest::Cue(seconds)) => assert!((seconds - 10.5).abs() < 0.05, "{seconds}"),
            other => panic!("asked {other:?}"),
        }
        view.rolling = true;
        pass(&ctx, &mut view, click(at));
        assert_eq!(view.request, None, "a cue while rolling would be cut at once");
    }
}
