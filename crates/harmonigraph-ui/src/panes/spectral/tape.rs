//! The Analyzer's Tape mode (#1719): the far region shows what the tape holds
//! along the song, as a spectrogram analyzed from the recording, and a click
//! there plays the recording from that spot while the transport is stopped.
//! Bitwig's own playhead cannot do this: a plugin is not told when it is
//! clicked while stopped (#1721).
//!
//! Only the editor's own copy of the pane draws it. The Video preview and an
//! export draw the live picture, which is all they have. The spectrum stays
//! live in Tape mode, so during an audition it shows the tape.
//!
//! With the roll on, the region also rolls the notes the recording holds,
//! each stretch's from the pass whose audio won it, and the notes played live
//! over the last audition at the song seconds they sounded over. Note names
//! label them by the live roll's rule (#1728): each note at the end that reads
//! first, which in the default orientation is its start.
//!
//! The song runs from its start at the far edge toward the spectrum, so a
//! recording made while the transport rolls grows toward the curve, the way
//! the live picture flows out from it. The region shows the whole recording
//! until it is zoomed (pinch or Ctrl+wheel) or panned (a depth drag, or a
//! sideways scroll where time runs across the pane); see `drag_zoom`.

use std::sync::Arc;

use egui::{Align, Align2, Pos2, Rect, Stroke};
use harmonigraph_core::spectrum::SPECTRUM_BINS;
use harmonigraph_render::{SpectrogramGrid, SpectrogramSlab, SpectrogramVertex};

use super::axes::{Axes, PitchScale, TimeAxis};
use super::roll::{self, RollDrawOptions, RollSource, Width};
use super::Navigation;
use crate::spectrogram::{pane_ppp, pitch_pixels, read_of, PaneView, LIVE_SLAB_CAP};
use crate::tape::spectrogram::{silent_slab, COLUMN_SECONDS, LEVELS};
use crate::tape::{Speed, TapeRequest, TapeView};
use crate::{theme, PictureState};

/// The narrowest stretch of song the region will zoom to, in seconds.
const MIN_SECONDS: f64 = 2.0;
/// The space a label needs along the time axis, in points, before the ruler
/// labels every bar or every few.
const LABEL_SPACING: f32 = 44.0;
/// The widest stretch of song the region will zoom out to: the widest slabs
/// the pyramid holds, no more of them than a live window is cut into. Past
/// it, the slabs drawn grow with the window rather than with the pane.
const MAX_SECONDS: f64 = COLUMN_SECONDS * (1u32 << (LEVELS - 1)) as f64 * LIVE_SLAB_CAP as f64;
/// The thickness, in points, of the band along the low-pitch edge that marks
/// what is recorded: the one place a gap reads apart from recorded silence.
const COVERAGE_PT: f32 = 3.0;
/// How much of the picture's ground is laid over a cold stretch: recorded
/// without the run-up its tails and bus state needed, so dimmed.
const COLD_DIM: f32 = 0.5;

/// The stretch of song the region shows. `zoomed` is set by any zoom or pan,
/// until Fit clears it; until then the region follows the recording's length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Window {
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
    pub(super) fn zoom(&mut self, factor: f64, at: f64) {
        let width = (self.seconds() / factor).clamp(MIN_SECONDS, MAX_SECONDS);
        let share = (at - self.from) / self.seconds();
        self.from = (at - share * width).max(0.0);
        self.to = self.from + width;
        self.zoomed = true;
    }

    pub(super) fn pan(&mut self, seconds: f64) {
        let shift = seconds.max(-self.from);
        self.from += shift;
        self.to += shift;
        self.zoomed = true;
    }
}

/// Song time over the far region: its start at the far edge, the newest at
/// the spectrum.
#[derive(Clone, Copy)]
pub(super) struct SongAxis {
    split: f32,
    window: Window,
    /// The region's length along the depth axis, in points.
    region_len: f32,
}

impl SongAxis {
    pub(super) fn new(axes: &Axes, split: f32, window: Window) -> SongAxis {
        SongAxis { split, window, region_len: axes.depth_len() * (1.0 - split) }
    }

    fn share(&self, d: f32) -> f64 {
        f64::from((d - self.split) / (1.0 - self.split).max(f32::EPSILON))
    }

    /// The depth song seconds `seconds` are drawn at, unclamped.
    fn depth_of(&self, seconds: f64) -> f32 {
        let share = (self.window.to - seconds) / self.window.seconds();
        self.split + share as f32 * (1.0 - self.split)
    }

    pub(super) fn seconds_at(&self, d: f32) -> f64 {
        self.window.to - self.share(d) * self.window.seconds()
    }

    /// Song seconds per point along the depth axis, the rate a drag or a
    /// scroll pans by.
    pub(super) fn seconds_per_point(&self) -> f64 {
        self.window.seconds() / f64::from(self.region_len.max(1.0))
    }

    fn shows(&self, seconds: f64) -> bool {
        (self.window.from..=self.window.to).contains(&seconds)
    }
}

/// Only the editor's own copy of the pane has a tape to show or a pointer to
/// cue it with.
pub(super) fn editor(navigation: Navigation) -> bool {
    matches!(navigation, Navigation::Docked | Navigation::Folded(_))
}

/// This copy of the pane draws the tape: it is the editor's, in Tape mode,
/// with a far region to draw it in.
pub(super) fn active(state: &PictureState, navigation: Navigation) -> bool {
    let view = &state.runtime.tape;
    view.present && view.showing && editor(navigation) && room(&state.appearance.spectrum)
}

/// The pane has a far region: the roll or the spectrogram is on.
fn room(cfg: &crate::SpectrumConfig) -> bool {
    cfg.show_roll || cfg.show_spectrogram
}

fn window_id(surface: usize) -> egui::Id {
    egui::Id::new(("tape-window", surface))
}

/// The stretch of song this frame starts from.
pub(super) fn window(ui: &egui::Ui, view: &TapeView, surface: usize) -> Window {
    let held = ui.data(|d| d.get_temp::<Window>(window_id(surface)));
    held.filter(|w| w.zoomed).unwrap_or_else(|| Window::fit(view))
}

/// Keep the stretch shown for the next frame, and tell the worker which
/// columns to analyze first.
pub(super) fn keep_window(ui: &egui::Ui, surface: usize, window: Window, view: &mut TapeView) {
    ui.data_mut(|d| d.insert_temp(window_id(surface), window));
    view.window = Some((window.from, window.to));
}

/// The tape's own GPU grid, apart from the live heatmap's, so that switching
/// modes re-uploads neither.
fn pane_id(surface: usize) -> u64 {
    u64::MAX - surface as u64
}

/// Seconds one slab of `level` spans.
fn slab_seconds(level: usize) -> f64 {
    COLUMN_SECONDS * f64::from(1u32 << level)
}

/// What the roll needs that the pane decides: the width it draws a note at,
/// the preview's point scale, and the display clock.
pub(super) struct RollFrame<'a> {
    pub(super) width: &'a Width,
    pub(super) point_scale: f32,
    pub(super) now: f64,
}

/// Draw the tape over the far region, and take a click there as a cue.
/// Returns the ruler's labels, for the pane to set with its own.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    painter: &egui::Painter,
    response: &egui::Response,
    axes: &Axes,
    scale: &PitchScale,
    state: &mut PictureState,
    split: f32,
    surface: usize,
    window: Window,
    frame: RollFrame,
) -> Vec<(Pos2, Align2, String)> {
    let axis = SongAxis::new(axes, split, window);
    heatmap(painter, axes, scale, state, &axis, surface);
    if state.appearance.spectrum.show_roll {
        notes(painter, axes, scale, state, &axis, surface, frame);
    }
    let view = &mut state.runtime.tape;
    for span in view.spans.iter().filter(|span| !span.warm) {
        let (from, to) = (span.start.max(window.from), span.end.min(window.to));
        if from < to {
            let band = Rect::from_two_pos(
                axes.at(0.0, axis.depth_of(from)),
                axes.at(1.0, axis.depth_of(to)),
            );
            painter.rect_filled(band, 0.0, theme::picture().gamma_multiply(COLD_DIM));
        }
    }
    // What is recorded, along the low-pitch edge under the ruler's ticks, as
    // the Tape tab's strip drew it.
    let up = axes.dir_pitch() * COVERAGE_PT;
    for span in &view.spans {
        let (from, to) = (span.start.max(window.from), span.end.min(window.to));
        if from < to {
            let edge = axes.at(0.0, axis.depth_of(from));
            let band = Rect::from_two_pos(edge, axes.at(0.0, axis.depth_of(to)) + up);
            let fill = if span.warm {
                theme::accent_fill()
            } else {
                theme::accent_fill().gamma_multiply(0.45)
            };
            painter.rect_filled(band, 0.0, fill);
        }
    }
    let labels = ruler(painter, axes, view, &axis);
    let across = |seconds: f64, stroke: Stroke| {
        if axis.shows(seconds) {
            painter.line_segment(axes.across_pitch(axis.depth_of(seconds)), stroke);
        }
    };
    if let Some(position) = view.position {
        across(position, Stroke::new(theme::LINE_WIDTH, theme::text_dim()));
    }
    if let Some(heard) = view.heard {
        across(heard, Stroke::new(theme::LINE_WIDTH, theme::accent()));
    }
    let pointer = response.hover_pos().filter(|&p| axes.depth_at(p) >= split);
    if let Some(pointer) = pointer {
        let at = axis.seconds_at(axes.depth_at(pointer)).max(0.0);
        across(at, Stroke::new(theme::GRID_WIDTH, theme::text()));
        // The pane senses drags only, so a drag takes hold on the press; a
        // press let go of before it travelled far enough to be a drag is a
        // click.
        let click = response.drag_stopped()
            && response.ctx.input(|i| i.pointer.could_any_button_be_click());
        if click && view.enabled && !view.rolling {
            view.request = Some(TapeRequest::Cue(at));
        }
        response.clone().on_hover_text_at_pointer(describe(view, at));
    }
    labels
}

/// The tape's spectrogram over the far region, at the pyramid level whose
/// slabs come closest to one per device pixel without going under it.
fn heatmap(
    painter: &egui::Painter,
    axes: &Axes,
    scale: &PitchScale,
    state: &mut PictureState,
    axis: &SongAxis,
    surface: usize,
) {
    let cfg = state.appearance.spectrum;
    let ppp = pane_ppp(painter.ctx());
    let window = axis.window;
    let target = f64::from((axis.region_len * ppp).clamp(2.0, LIVE_SLAB_CAP));
    let level =
        (0..LEVELS).find(|&n| window.seconds() / slab_seconds(n) <= target).unwrap_or(LEVELS - 1);
    let width = slab_seconds(level);
    // A slab either side, so the filtered edge of the run is past the region.
    let first = (window.from / width).floor() as i64 - 1;
    let last = (window.to / width).ceil() as i64 + 1;
    let tape = &state.runtime.tape.spectrogram;
    let run: Arc<[SpectrogramSlab]> =
        (first..=last).map(|j| tape.slab(level, j).cloned().unwrap_or_else(silent_slab)).collect();
    // A power of two over the run, so the ring keeps its size while a pan or
    // a zoom within one level moves the run.
    let capacity = ((run.len() + 2).next_power_of_two() + 8) as u32;
    // Column k is centred on its own song seconds, so slab j of any level
    // starts half a column before column `j * 2^level`.
    let origin = first as f64 * width - 0.5 * COLUMN_SECONDS;
    let vert = |p: f32, d: f32| {
        let pos = axes.at(p, d);
        let slab = ((axis.seconds_at(d) - origin) / width) as f32;
        SpectrogramVertex { pos: [pos.x, pos.y], slab, t: p }
    };
    let (low_far, high_far) = (vert(0.0, 1.0), vert(1.0, 1.0));
    let (high_near, low_near) = (vert(1.0, axis.split), vert(0.0, axis.split));
    let vertices = vec![low_far, high_far, high_near, low_far, high_near, low_near];
    let view = PaneView {
        ppp,
        pitch_len: axes.pitch_len(),
        depth_len: axis.region_len,
        window: window.seconds(),
        scale: *scale,
        cfg,
    };
    let grid = SpectrogramGrid {
        capacity,
        bins: SPECTRUM_BINS as u32,
        first_key: first,
        run,
        full_uploads: Default::default(),
    };
    let shades = crate::spectrogram::shades(&mut state.surfaces.spectrogram, surface, &cfg);
    painter.add(harmonigraph_render::spectrogram_paint_callback(
        painter.clip_rect(),
        vertices,
        grid,
        read_of(&view, pitch_pixels(axes.pitch_len(), ppp)),
        shades,
        state.surfaces.target_format,
        pane_id(surface),
        None,
    ));
}

/// The recording's notes along the song, and over them the notes played live
/// during the last audition, at the song seconds they sounded over.
fn notes(
    painter: &egui::Painter,
    axes: &Axes,
    scale: &PitchScale,
    state: &PictureState,
    axis: &SongAxis,
    surface: usize,
    frame: RollFrame,
) {
    let sources = roll_sources(state, axis.split, axis.window, frame.now);
    let options = RollDrawOptions {
        split: axis.split,
        now: frame.now,
        surface,
        point_scale: frame.point_scale,
    };
    roll::draw_rolls(painter, axes, scale, frame.width, state, &options, &sources);
}

/// The rolls Tape mode draws and names over `window`: the recording's notes,
/// and the notes played live over the last audition. `now` is the display
/// clock.
pub(super) fn roll_sources<'a>(
    state: &'a PictureState,
    split: f32,
    window: Window,
    now: f64,
) -> Vec<RollSource<'a>> {
    let view = &state.runtime.tape;
    // Every recorded note has ended: a stretch's notes close where it does.
    let song = TimeAxis {
        split,
        depth_span: 1.0 - split,
        window: window.seconds(),
        origin: window.to,
        now: window.to,
    };
    let recorded = RollSource { roll: &view.notes, time: song, lead: false, live_during: None };
    let mut sources = vec![recorded];
    if let Some(audition) = view.audition {
        // The display clock at the window's near edge, and how long that
        // clock took to cross the window: longer for a slowed audition.
        let origin = audition.clock_at(window.to);
        let seconds = audition.clock_at(window.to) - audition.clock_at(window.from);
        sources.push(RollSource {
            roll: state.roll(),
            time: TimeAxis { origin, window: seconds, now, ..song },
            lead: false,
            live_during: Some((audition.clock, audition.until.unwrap_or(f64::INFINITY))),
        });
    }
    sources
}

/// Bars, or minutes and seconds with no meter from the host, ticked along the
/// low-pitch edge: ticks rather than lines across, which would lay a mesh over
/// the picture they measure.
fn ruler(
    painter: &egui::Painter,
    axes: &Axes,
    view: &TapeView,
    axis: &SongAxis,
) -> Vec<(Pos2, Align2, String)> {
    let window = axis.window;
    let pitch = axes.dir_pitch();
    let later = -axes.dir_depth();
    // The label box grows away from its tick, into the picture.
    let grows = later + pitch;
    let align = Align2([
        if grows.x >= 0.0 { Align::Min } else { Align::Max },
        if grows.y >= 0.0 { Align::Min } else { Align::Max },
    ]);
    let mut labels = Vec::new();
    let mut tick = |seconds: f64, text: Option<String>| {
        let edge = axes.at(0.0, axis.depth_of(seconds));
        let length = if text.is_some() { 12.0 } else { 6.0 };
        painter.line_segment(
            [edge, edge + pitch * length],
            Stroke::new(theme::GRID_WIDTH, theme::picture_marking()),
        );
        if let Some(text) = text {
            labels.push((edge + later * 3.0 + pitch * 3.0, align, text));
        }
    };
    let per_second = (1.0 / axis.seconds_per_point()) as f32;
    if let Some(bar) = view.bar_seconds(window.from) {
        let per_bar = (bar as f32 * per_second).max(f32::EPSILON);
        let every = [1, 2, 4, 8, 16, 32, 64, 128, 256, 512]
            .into_iter()
            .find(|n| per_bar * *n as f32 >= LABEL_SPACING)
            .unwrap_or(1024);
        for (seconds, bar) in view.bar_lines(window.from, window.to) {
            let labelled = (bar - 1) % every == 0;
            if labelled || per_bar >= 4.0 {
                tick(seconds, labelled.then(|| bar.to_string()));
            }
        }
        return labels;
    }
    let every = [1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0]
        .into_iter()
        .find(|n| per_second * *n as f32 >= LABEL_SPACING * 1.5)
        .unwrap_or(1200.0);
    let mut seconds = (window.from / every).ceil() * every;
    while seconds < window.to {
        tick(seconds, Some(clock(seconds).trim_end_matches(".0").to_owned()));
        seconds += every;
    }
    labels
}

/// Live or Tape, and in Tape mode the audition's controls, in the far
/// region's corner beside its fold control. `song` is the stretch shown, in
/// Tape mode.
///
/// With no far region it sits in the pane's own corner, where the switch says
/// what would make room for the tape.
pub(super) fn bar(
    ui: &mut egui::Ui,
    axes: &Axes,
    state: &mut PictureState,
    split: f32,
    song: Option<&mut Window>,
) {
    let ui_scale = theme::ui_scale(ui.ctx());
    let size = theme::tab_bar_height(ui_scale);
    // With no far region (`split` 1), the whole pane.
    let near = if split < 1.0 { split } else { 0.0 };
    let region = Rect::from_two_pos(axes.at(0.0, near), axes.at(1.0, 1.0));
    let corner = axes.at(1.0, 1.0);
    let right = (corner.x - region.right()).abs() < (corner.x - region.left()).abs();
    let bottom = (corner.y - region.bottom()).abs() < (corner.y - region.top()).abs();
    let gap = 4.0;
    let (x0, x1) = if right {
        (region.left() + gap, region.right() - size - gap)
    } else {
        (region.left() + size + gap, region.right() - gap)
    };
    let (y0, y1) = if bottom {
        (region.bottom() - size, region.bottom())
    } else {
        (region.top(), region.top() + size)
    };
    let rect = Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1.max(x0), y1));
    let layout = if right {
        egui::Layout::right_to_left(Align::Center)
    } else {
        egui::Layout::left_to_right(Align::Center)
    };
    // The whole bar answers the pointer, not only its buttons: a click on its
    // padding or its text is not a cue, nor a scroll over it a pan. Laid
    // under the buttons at last frame's size, which is where the pointer saw
    // it, so they stay on top.
    let id = ui.id().with("tape-bar");
    let held = ui.data(|d| d.get_temp::<Rect>(id));
    if let Some(held) = held {
        ui.interact(held, id.with("under"), egui::Sense::click_and_drag());
    }
    let mut child =
        ui.new_child(egui::UiBuilder::new().id_salt("tape-bar").max_rect(rect).layout(layout));
    let room = room(&state.appearance.spectrum);
    let drawn = egui::Frame::new()
        .fill(theme::header())
        .corner_radius(theme::control_radius(ui_scale))
        .inner_margin(egui::Margin::symmetric(4, 1))
        .show(&mut child, |ui| controls(ui, &mut state.runtime.tape, song, room));
    ui.data_mut(|d| d.insert_temp(id, drawn.response.rect));
}

fn controls(ui: &mut egui::Ui, view: &mut TapeView, song: Option<&mut Window>, room: bool) {
    ui.horizontal(|ui| {
        let live = ui.add(egui::Button::selectable(!view.showing || !room, "Live"));
        if live.on_hover_text("The live picture.").clicked() {
            view.showing = false;
        }
        let tape = ui.add_enabled(room, egui::Button::selectable(view.showing && room, "Tape"));
        let hover = "What the tape holds along the song.";
        let disabled = "Turn on the spectrogram or the roll to make room for the tape.";
        if tape.on_hover_text(hover).on_disabled_hover_text(disabled).clicked() {
            view.showing = true;
        }
    });
    let Some(song) = song else { return };
    let stop = ui.add_enabled(view.heard.is_some(), egui::Button::new("Stop"));
    if stop.on_hover_text("End the audition.").clicked() {
        view.request = Some(TapeRequest::Stop);
    }
    let combo =
        egui::ComboBox::from_id_salt("tape-speed").selected_text(view.speed.label()).width(0.0);
    let speed = crate::widgets::selected_combo(ui, combo, |ui| {
        for speed in Speed::ALL {
            ui.selectable_value(&mut view.speed, speed, speed.label());
        }
    });
    speed.response.on_hover_text(
        "How fast the audition plays, its pitch held. Changing it while the tape plays \
         starts again from the spot heard. The slowest speed blurs attacks.",
    );
    let fit = ui.add_enabled(song.zoomed, egui::Button::new("Fit"));
    if fit.on_hover_text("Show the whole recording.").clicked() {
        *song = Window::fit(view);
    }
    ui.label(format!("{} MiB", view.bytes >> 20))
        .on_hover_text("Memory the recording holds. It is never saved with the project.");
    ui.label(egui::RichText::new(readout(view)).small().color(theme::text_dim()));
}

/// The bar's one line (#1719 slice 4): why the tape cannot sound, or what
/// sounds, or what is recorded where the transport stopped. Bitwig reports
/// no playhead moved while stopped, so that is where playback stopped.
fn readout(view: &TapeView) -> String {
    if !view.enabled {
        return "Off: the analyzer listens to the sidechain, or the host is rendering.".to_owned();
    }
    if view.rolling {
        return "Recording. Stop the transport to audition.".to_owned();
    }
    if view.spans.is_empty() {
        return "Nothing recorded yet. The song records as it plays.".to_owned();
    }
    match (view.heard, view.position) {
        (Some(heard), _) => {
            // The cue's own speed, which a new one reaches only once it cues
            // again.
            let sounding = view.audition.map_or(1.0, |audition| audition.speed);
            let speed = Speed::ALL
                .into_iter()
                .find(|speed| speed.factor() == sounding && *speed != Speed::Full)
                .map_or_else(String::new, |speed| format!(" at {}", speed.label()));
            format!("Playing {}{speed} · {}", place(view, heard), held(view, heard))
        }
        (None, Some(at)) => {
            format!("Stopped at {} · {} · Click to play.", place(view, at), held(view, at))
        }
        (None, None) => "Click to play the recording from there.".to_owned(),
    }
}

/// Song seconds as `m:ss.s`.
fn clock(seconds: f64) -> String {
    let minutes = (seconds / 60.0).floor();
    format!("{minutes:.0}:{:04.1}", seconds - minutes * 60.0)
}

/// The hover text for song seconds `at`.
fn describe(view: &TapeView, at: f64) -> String {
    format!("{}\n{}", place(view, at), held(view, at))
}

/// Song seconds `at` as the host's bar and beat, and as a clock.
fn place(view: &TapeView, at: f64) -> String {
    match view.bar_at(at) {
        Some((bar, beat)) => format!("Bar {bar}.{beat} · {}", clock(at)),
        None => clock(at),
    }
}

/// What is recorded at song seconds `at`: warm or cold, and how long ago.
fn held(view: &TapeView, at: f64) -> String {
    match view.span_at(at) {
        Some(span) => {
            let ago = span.recorded.elapsed().as_secs_f64();
            let ago =
                if ago < 90.0 { format!("{ago:.0} s") } else { format!("{:.0} min", ago / 60.0) };
            format!("{}, recorded {ago} ago", if span.warm { "warm" } else { "cold" })
        }
        None => "nothing recorded".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_says_why_the_tape_is_silent_or_what_is_recorded_where_it_plays() {
        let span = crate::tape::TapeSpan {
            start: 0.0,
            end: 40.0,
            warm: false,
            recorded: std::time::Instant::now(),
        };
        let mut view = TapeView {
            present: true,
            spans: vec![span],
            position: Some(5.0),
            ..Default::default()
        };
        assert!(readout(&view).starts_with("Off: "), "{}", readout(&view));
        view.enabled = true;
        let parked = readout(&view);
        assert!(parked.starts_with("Stopped at 0:05.0 · cold, recorded "), "{parked}");
        view.heard = Some(12.5);
        let playing = readout(&view);
        assert!(playing.starts_with("Playing 0:12.5 · cold"), "{playing}");
        view.speed = Speed::Half;
        let asked = readout(&view);
        assert!(asked.starts_with("Playing 0:12.5 · cold"), "not yet cued again: {asked}");
        let audition = crate::tape::Audition { clock: 0.0, song: 12.0, speed: 0.5, until: None };
        view.audition = Some(audition);
        let slowed = readout(&view);
        assert!(slowed.starts_with("Playing 0:12.5 at 0.5× · cold"), "{slowed}");
        view.rolling = true;
        assert!(readout(&view).starts_with("Recording."), "{}", readout(&view));
    }

    #[test]
    fn zooming_out_stops_where_the_widest_slabs_fill_a_live_window() {
        let mut window = Window { from: 0.0, to: 60.0, zoomed: false };
        for _ in 0..100 {
            window.zoom(0.5, 30.0);
        }
        assert_eq!(window.seconds(), MAX_SECONDS);
    }
}
