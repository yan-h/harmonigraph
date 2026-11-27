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
//! until it is zoomed (pinch or Ctrl+wheel; see `drag_zoom`) or moved, by a
//! sideways scroll where time runs across the pane or on the overview: a strip
//! under the pane holding the whole recording, with the stretch shown boxed.
//! A drag in the region itself moves nothing, which keeps it free to mean
//! something of its own.

use egui::{Align, Align2, Pos2, Rect, Stroke};
use harmonigraph_render::SpectrogramVertex;

use super::axes::{Axes, PitchScale, TimeAxis};
use super::roll::{self, RollDrawOptions, RollSource, Width};
use super::Navigation;
use crate::spectrogram::{
    pane_ppp, pitch_pixels, read_of, PaneView, TapeRun, TapeStrip, LIVE_SLAB_CAP,
};
use crate::tape::spectrogram::{silent_slab, COLUMN_SECONDS, LEVELS};
use crate::tape::{Speed, TapeRequest, TapeView};
use crate::{theme, PictureState};

/// The narrowest stretch of song the region will zoom to, in seconds.
const MIN_SECONDS: f64 = 2.0;
/// The space a label needs along the time axis, in points, before the ruler
/// labels every bar or every few.
const LABEL_SPACING: f32 = 44.0;
/// How long a playhead may stand at one value and still count as advancing.
/// The host's position and the audition's both reach the editor once per
/// audio block (about 11 ms at 512 frames and 48 kHz, more for larger
/// blocks), so at 144 Hz several frames in a row can see the same value
/// while the playhead runs on; a still frame there is no stall. A value held
/// longer than this — a stalled input worker (AGENTS.md's stakes record) —
/// stops holding the loop rather than drawing a still mark at it.
const PLAYHEAD_STALL_SECONDS: f64 = 0.25;
/// The thickness, in points, of the band along the low-pitch edge that marks
/// what is recorded: the one place a gap reads apart from recorded silence.
const COVERAGE_PT: f32 = 3.0;
/// How much of the picture's ground is laid over a cold stretch: recorded
/// without the run-up its tails and bus state needed, so dimmed.
const COLD_DIM: f32 = 0.5;

/// The stretch of song the region shows. `zoomed` is set by any zoom or pan
/// that moves it, until a double-click on the overview clears it; until then
/// the region follows the recording's length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Window {
    from: f64,
    to: f64,
    zoomed: bool,
    /// The overview strip's end, which no move carries the stretch past
    /// unless it is wider than the strip, when it starts at the song's start:
    /// past it the box leaves the strip, where nothing shows the stretch or
    /// can take hold of it. Set as each frame reads the window.
    end: f64,
}

impl Window {
    /// The whole recording, the reported position while it rolls and what
    /// sounds, with a little room past the end. A stopped position is left
    /// out: Bitwig keeps reporting where the transport stopped, which after a
    /// transport left rolling past the music is far past anything recorded.
    fn fit(view: &TapeView) -> Window {
        Window::fitted(Window::reach(view, 0.0))
    }

    /// The end [`fit`](Self::fit) makes room for, `ahead` seconds on: the
    /// recording's, and each playhead's moved on at the rate it runs
    /// ([`rates`]).
    fn reach(view: &TapeView, ahead: f64) -> f64 {
        let (position_rate, heard_rate) = rates(view);
        let position = view.position.zip(position_rate).map(|(at, rate)| at + rate * ahead);
        let heard = view.heard.map(|at| at + heard_rate * ahead);
        [view.end(), position, heard].into_iter().flatten().fold(0.0, f64::max)
    }

    /// The stretch a fit to `end` shows.
    fn fitted(end: f64) -> Window {
        let to = (end * 1.05).max(MIN_SECONDS * 4.0);
        Window { from: 0.0, to, zoomed: false, end: to }
    }

    fn seconds(&self) -> f64 {
        self.to - self.from
    }

    /// Zoom by `factor` (above 1 zooms in) about song seconds `at`, out to
    /// the whole strip at most.
    pub(super) fn zoom(&mut self, factor: f64, at: f64) {
        let width = (self.seconds() / factor).clamp(MIN_SECONDS, self.end);
        let share = (at - self.from) / self.seconds();
        self.from = self.on_strip(at - share * width, width);
        self.to = self.from + width;
        self.zoomed = true;
    }

    /// A start for a stretch `width` long, as near `from` as the song's start
    /// and the strip's end allow.
    fn on_strip(&self, from: f64, width: f64) -> f64 {
        from.clamp(0.0, (self.end - width).max(0.0))
    }

    /// This stretch moved by `seconds`, its width kept, as far as the song's
    /// start and the strip's end allow; itself where neither lets it move.
    pub(super) fn moved(self, seconds: f64) -> Window {
        let from = self.on_strip(self.from + seconds, self.seconds());
        if from == self.from {
            return self;
        }
        Window { from, to: from + self.seconds(), zoomed: true, ..self }
    }

    /// The same width, centred on song seconds `at` as nearly as the song's
    /// start and the strip's end allow.
    fn centred(self, at: f64) -> Window {
        self.moved(at - 0.5 * self.seconds() - self.from)
    }

    /// Its start moved to `from`, a point on the strip, its end held, and no
    /// narrower than the zoom allows.
    fn with_from(self, from: f64) -> Window {
        let from = from.min(self.to - MIN_SECONDS);
        Window { from, zoomed: true, ..self }
    }

    /// Its end moved to `to`, a point on the strip, its start held, and no
    /// narrower than the zoom allows.
    fn with_to(self, to: f64) -> Window {
        let to = to.max(self.from + MIN_SECONDS);
        Window { to, zoomed: true, ..self }
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

    /// Song seconds per point along the depth axis, the rate a scroll pans
    /// by.
    pub(super) fn seconds_per_point(&self) -> f64 {
        self.window.seconds() / f64::from(self.region_len.max(1.0))
    }

    fn shows(&self, seconds: f64) -> bool {
        (self.window.from..=self.window.to).contains(&seconds)
    }

    /// Where song seconds `seconds` stand along the region, in points from
    /// its near edge.
    fn along(&self, seconds: f64) -> f64 {
        (self.window.to - seconds) / self.window.seconds() * f64::from(self.region_len)
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
    view.present && view.showing && editor(navigation) && state.appearance.spectrum.has_far_region()
}

fn window_id(surface: usize) -> egui::Id {
    egui::Id::unique(("tape-window", surface))
}

/// The stretch of song this frame starts from.
pub(super) fn window(ui: &egui::Ui, view: &TapeView, surface: usize) -> Window {
    let fit = Window::fit(view);
    let held = ui.data(|d| d.get_temp::<Window>(window_id(surface)));
    // Onto the strip as it is now, which shrinks where the reported position
    // or what sounds falls back.
    held.filter(|w| w.zoomed).map_or(fit, |w| Window { end: fit.end, ..w }.moved(0.0))
}

/// Keep the stretch shown for the next frame.
pub(super) fn keep_window(ui: &egui::Ui, surface: usize, window: Window) {
    ui.data_mut(|d| d.insert_temp(window_id(surface), window));
}

/// The stretch shown on `surface` by the last frame drawn in Tape mode.
#[cfg(test)]
pub(super) fn kept_window(ctx: &egui::Context, surface: usize) -> Option<(f64, f64)> {
    ctx.data(|d| d.get_temp::<Window>(window_id(surface))).map(|w| (w.from, w.to))
}

/// The GPU grid `strip` draws on `surface`, apart from the live heatmap's
/// and from the other strip's: switching modes re-uploads neither heatmap,
/// and moving the overview's box re-uploads neither strip.
fn grid_id(strip: TapeStrip, surface: usize) -> u64 {
    // The overview's count down below every far region's, which count down
    // from the top one per surface.
    const BELOW_PANES: u64 = 1 << 16;
    match strip {
        TapeStrip::Region => u64::MAX - surface as u64,
        TapeStrip::Overview => u64::MAX - BELOW_PANES - surface as u64,
    }
}

/// Seconds one slab of `level` spans.
fn slab_seconds(level: usize) -> f64 {
    COLUMN_SECONDS * f64::from(1u32 << level)
}

/// This frame lets go of the click that cues the tape: over the song, where a
/// cue can start. The pane senses drags only, so a drag takes hold on the
/// press; a press let go of before it travelled far enough to be a drag is a
/// click.
fn cues(response: &egui::Response, axes: &Axes, split: f32, view: &TapeView) -> bool {
    response.drag_stopped()
        && response.ctx.input(|i| i.pointer.could_any_button_be_click())
        && view.enabled
        && !view.rolling
        && response.hover_pos().is_some_and(|p| axes.depth_at(p) >= split)
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
    heatmap(painter, axes, scale, state, &axis, surface, TapeStrip::Region);
    if state.appearance.spectrum.show_roll {
        // Through the live roll's own rule, which holds nothing for a song
        // axis: it stands still, and a live note played over the audition
        // moves only while it sounds, which its voice holds. Both sources are
        // built without a now line, which alone scrolls a roll or grows a lead.
        let motion = notes(painter, axes, scale, state, &axis, surface, frame);
        motion.hold(&mut state.surfaces.moving_this_frame, 0.0);
    }
    let view = &mut state.runtime.tape;
    coverage(painter, axes, view, &axis);
    let labels = ruler(painter, axes, view, &axis);
    let pixel = marks(painter, axes, view, &axis, egui::Id::unique(("tape-marks", surface)));
    if let Some(pixel) = pixel {
        state.surfaces.moving_this_frame.hold_for(crate::Reason::Playhead, pixel);
    }
    let view = &mut state.runtime.tape;
    let pointer = response.hover_pos().filter(|&p| axes.depth_at(p) >= split);
    if let Some(pointer) = pointer {
        let at = axis.seconds_at(axes.depth_at(pointer)).max(0.0);
        if axis.shows(at) {
            painter.line_segment(
                axes.across_pitch(axis.depth_of(at)),
                Stroke::new(theme::GRID_WIDTH, theme::text()),
            );
        }
        if cues(response, axes, split, view) {
            view.request = Some(TapeRequest::Cue(at));
        }
        response.clone().on_hover_text_at_pointer(describe(view, at));
    }
    labels
}

/// What is recorded over `axis`: cold stretches dimmed, and a band along the
/// low-pitch edge under the ruler's ticks.
fn coverage(painter: &egui::Painter, axes: &Axes, view: &TapeView, axis: &SongAxis) {
    let window = axis.window;
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
}

/// The song seconds per second each playhead runs at: the reported position
/// while the transport rolls (`None` while it stands), and the audition at the
/// speed it was cued at, which a new choice of speed reaches only on its next
/// cue (as the readout says).
fn rates(view: &TapeView) -> (Option<f64>, f64) {
    (view.rolling.then_some(1.0), view.audition.map_or(1.0, |audition| audition.speed))
}

/// What each playhead's mark last showed, and when that last changed, by the
/// egui clock: whether it still advances.
#[derive(Clone, Copy, Default)]
struct Seen {
    position: Option<(f64, f64)>,
    heard: Option<(f64, f64)>,
}

/// The reported position (dim) and the audition's playhead (accent) over
/// `axis`, the one painter both the pane and the overview draw them with.
///
/// Answers when a mark it drew next moves a device pixel, in seconds, for the
/// frame loop ([`Reason::Playhead`](crate::Reason::Playhead)), by one rule for
/// both: on the stretch shown, still advancing (its value changed within
/// [`PLAYHEAD_STALL_SECONDS`], kept under `seen`), and moving on the axis as it
/// moves ([`screen_speed`]); the sooner of the two marks (#1911). Not the
/// transport rolling or an audition playing as such, which would hold the
/// editor with the playhead off the stretch shown or stalled (#1837).
fn marks(
    painter: &egui::Painter,
    axes: &Axes,
    view: &TapeView,
    axis: &SongAxis,
    seen: egui::Id,
) -> Option<f64> {
    let ctx = painter.ctx();
    let clock = ctx.input(|i| i.time);
    let ppp = ctx.pixels_per_point();
    let mut last = ctx.data(|d| d.get_temp::<Seen>(seen)).unwrap_or_default();
    let (position_rate, heard_rate) = rates(view);
    let mut pixel = None::<f64>;
    let mut mark = |at: Option<f64>, rate: Option<f64>, last: &mut Option<(f64, f64)>, stroke| {
        let moving = advancing(last, at, clock);
        let Some(at) = at else { return };
        if !axis.shows(at) {
            return;
        }
        painter.line_segment(axes.across_pitch(axis.depth_of(at)), stroke);
        let speed = rate.map_or(0.0, |rate| screen_speed(view, axis, at, rate, ppp));
        if moving && speed > 0.0 {
            pixel = Some(pixel.map_or(1.0 / speed, |sooner| sooner.min(1.0 / speed)));
        }
    };
    let dim = Stroke::new(theme::LINE_WIDTH, theme::text_dim());
    mark(view.position, position_rate, &mut last.position, dim);
    let accent = Stroke::new(theme::LINE_WIDTH, theme::accent());
    mark(view.heard, Some(heard_rate), &mut last.heard, accent);
    ctx.data_mut(|d| d.insert_temp(seen, last));
    pixel
}

/// A playhead at `at` is still advancing: its value changed within
/// [`PLAYHEAD_STALL_SECONDS`] of `clock`. `last` keeps its value and when that
/// last changed. A value seen for the first time counts as just changed, so a
/// cue's first frame already asks for the next one. Forgotten with the
/// playhead, so one that comes back on the value it left on is seen afresh.
fn advancing(last: &mut Option<(f64, f64)>, at: Option<f64>, clock: f64) -> bool {
    let Some(at) = at else {
        *last = None;
        return false;
    };
    let changed = match *last {
        Some((value, since)) if value == at => since,
        _ => clock,
    };
    *last = Some((at, changed));
    clock - changed <= PLAYHEAD_STALL_SECONDS
}

/// How fast a mark at song seconds `at`, running at `rate`, crosses the
/// screen, in device pixels a second: on the axis as it moves, from the rates
/// the playheads run at rather than from where the mark was last frame (a step
/// measured over the idle poll's gap is no measure of one at the display
/// rate). A stretch zoomed or panned to stands still; a fitted one grows with
/// whatever reaches its end, so a playhead that sets the end, as the
/// position does while it records, stands still on screen while the song
/// moves under it.
fn screen_speed(view: &TapeView, axis: &SongAxis, at: f64, rate: f64, ppp: f32) -> f64 {
    const AHEAD: f64 = 0.1;
    let later =
        if axis.window.zoomed { axis.window } else { Window::fitted(Window::reach(view, AHEAD)) };
    let then = SongAxis { window: later, ..*axis };
    (then.along(at + rate * AHEAD) - axis.along(at)).abs() / AHEAD * f64::from(ppp)
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
    strip: TapeStrip,
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
    let key = TapeRun { level, first, last, revision: tape.revision() };
    // In the live heatmap's one size of ring, which holds any run a region's
    // pixels cut, so neither a pan nor a zoom reallocates it: sized from the
    // run, a drag at an unlucky zoom re-sent the whole grid at every slab it
    // crossed.
    let grid = state.surfaces.spectrogram.at(surface).tape[strip as usize]
        .tape(key, |j| tape.slab(level, j).cloned().unwrap_or_else(silent_slab));
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
    let shades = crate::spectrogram::shades(&mut state.surfaces.spectrogram, surface, &cfg);
    painter.add(harmonigraph_render::spectrogram_paint_callback(
        painter.clip_rect(),
        vertices,
        grid,
        read_of(&view, pitch_pixels(axes.pitch_len(), ppp)),
        shades,
        state.surfaces.target_format,
        grid_id(strip, surface),
        None,
    ));
}

/// The recording's notes along the song, and over them the notes played live
/// during the last audition, at the song seconds they sounded over. Answers as
/// [`roll::draw_rolls`] does.
fn notes(
    painter: &egui::Painter,
    axes: &Axes,
    scale: &PitchScale,
    state: &PictureState,
    axis: &SongAxis,
    surface: usize,
    frame: RollFrame,
) -> roll::RollMotion {
    let sources = roll_sources(state, axis.split, axis.window, frame.now);
    let options = RollDrawOptions {
        split: axis.split,
        now: frame.now,
        surface,
        point_scale: frame.point_scale,
    };
    roll::draw_rolls(painter, axes, scale, frame.width, state, &options, &sources)
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
    let recorded = RollSource { roll: &view.notes, time: song, now_line: false, live_during: None };
    let mut sources = vec![recorded];
    if let Some(audition) = view.audition {
        // The display clock at the window's near edge, and how long that
        // clock took to cross the window: longer for a slowed audition.
        let origin = audition.clock_at(window.to);
        let seconds = audition.clock_at(window.to) - audition.clock_at(window.from);
        sources.push(RollSource {
            roll: state.roll(),
            time: TimeAxis { origin, window: seconds, now, ..song },
            now_line: false,
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

/// The audition's controls and its one line, on a row of their own along the
/// pane's bottom, over the overview: the whole width, where the header shares
/// its width with the tabs, and chrome rather than a bar over the picture, so
/// none of the tape is hidden under it and nothing it catches can reach the
/// picture as a cue. With no far region to draw the tape in, the line says
/// what would make room for it.
pub(super) fn controls(ui: &mut egui::Ui, rect: Rect, state: &mut PictureState) {
    let room = state.appearance.spectrum.has_far_region();
    let view = &mut state.runtime.tape;
    let ui_scale = theme::ui_scale(ui.ctx());
    ui.painter().rect_filled(rect, 0.0, theme::header());
    let inner = rect.shrink2(egui::vec2(theme::button_gap(ui_scale), 0.0));
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("tape-controls")
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    row.set_clip_rect(rect.intersect(ui.clip_rect()));
    row.spacing_mut().item_spacing.x = theme::button_gap(ui_scale);
    let ui = &mut row;
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
    // The memory at the row's far end, and the line in whatever room is
    // between, cut short where it runs out; a cut line shows itself whole on
    // hover.
    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
        ui.label(format!("{} MiB", view.bytes >> 20))
            .on_hover_text("Memory the recording holds. It is never saved with the project.");
        ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
            let line = if room {
                readout(view)
            } else {
                "Turn on the spectrogram or the roll to make room for the tape.".to_owned()
            };
            let line = egui::RichText::new(line).small().color(theme::text_dim());
            ui.add(egui::Label::new(line).truncate());
        });
    });
}

/// The heights of the controls' row and the overview under it, in points:
/// a header's, and enough for the recording's spectrogram to read as one.
pub(super) fn rows(ui_scale: f32) -> (f32, f32) {
    let row = theme::tab_bar_height(ui_scale);
    (row, 1.5 * row)
}

/// Span's bar, in beats.
pub(crate) const HARMONY_SPAN_RANGE: std::ops::RangeInclusive<f32> = 0.5..=16.0;

/// How the harmony row (#1974) cuts the song into sections; see
/// [`crate::tape::harmony`]. An editor preference
/// ([`EditorPrefs`](crate::EditorPrefs)) rather than part of the look: the row
/// is the editor's alone and never in a video, so a take has no use for it.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct HarmonyRow {
    pub show: bool,
    /// 0..=1: how small a change of harmony starts a section.
    pub detail: f32,
    /// Beats: how long a run of notes is heard as one harmony.
    pub span: f32,
}

impl Default for HarmonyRow {
    /// The settings the prototype's two test pieces read best at (#1974): a
    /// threshold of 0.06 and, at 120 beats a minute, a two-second Span.
    fn default() -> Self {
        HarmonyRow { show: true, detail: 0.5, span: 4.0 }
    }
}

impl HarmonyRow {
    /// Each bar reads out what the sections are cut by.
    pub(crate) fn sanitize(&mut self) {
        let fresh = HarmonyRow::default();
        self.detail = harmonigraph_scene::repair(self.detail, fresh.detail, 0.0..=1.0);
        self.span = harmonigraph_scene::repair(self.span, fresh.span, HARMONY_SPAN_RANGE);
    }
}

/// The harmony row's height, in points: twice the overview's, room for a
/// lattice six nodes tall at a size its rings still read at.
pub(super) fn harmony_height(ui_scale: f32) -> f32 {
    3.0 * theme::tab_bar_height(ui_scale)
}

/// How long a note struck under the playhead takes to settle from white to
/// its own colour, in song seconds.
const FLASH_SECONDS: f64 = 0.5;
/// A flash this many times [`FLASH_SECONDS`] old has settled: under a
/// hundredth of it is left.
const FLASH_SETTLED: f64 = 5.0;
/// How far a sevenths step is drawn from its fifths-and-thirds node, in node
/// spacings along each axis: the lattice's cabinet view, flattened.
const SEVENTH_OFFSET: f32 = 0.35;
/// A thumbnail's node spacing at most, as a share of the row's height, so a
/// song on one line of fifths does not draw three giant discs.
const NODE_SHARE: f32 = 0.28;

/// Where a lattice position sits in a thumbnail, in node spacings: thirds
/// (steps of five) run right and fifths (steps of three) run up, as the
/// lattice's fresh view has them.
fn plan(pos: harmonigraph_core::LatticePos) -> egui::Vec2 {
    let seventh = SEVENTH_OFFSET * pos.sevens as f32;
    egui::vec2(pos.fives as f32 + seventh, -(pos.threes as f32 + seventh))
}

/// Which node each pitch class of a recording lights, and the frame that holds
/// them all: kept from frame to frame, since finding them walks the naming
/// reach once per class, 1,025 nodes in the fresh look, which measured about
/// 0.1 ms a frame for a short piece (PR #1975).
///
/// Kept for exactly what it is worked out from: the analysis's pitch classes,
/// the tuning and its tempered commas, and the reach. A Detail or Span drag
/// cuts the song afresh over the same classes, so it is no new placement.
#[derive(Clone)]
struct Placed {
    /// The analysis last placed, by identity, so a frame that shows the same
    /// one need not read its classes again. Weak: the editor holds no
    /// analysis the worker has displaced, whose free stays off the lock.
    harmony: std::sync::Weak<crate::tape::harmony::Harmony>,
    /// Its pitch classes in microcents, ascending.
    classes: Vec<u32>,
    tuning: harmonigraph_core::Tuning,
    tempered: harmonigraph_core::Tempered,
    reach: harmonigraph_scene::DrawnWindow,
    /// By pitch class in microcents: two tunings of one note, a recorded
    /// 12-TET part beside one in the lattice's tuning, are two classes, and
    /// each lights its own node or none.
    lit: std::sync::Arc<std::collections::BTreeMap<u32, Option<harmonigraph_core::LatticePos>>>,
    /// The lit nodes' bounds, in [`plan`]'s node spacings.
    frame: Option<Rect>,
}

impl Placed {
    fn of(
        ctx: &egui::Context,
        id: egui::Id,
        harmony: &std::sync::Arc<crate::tape::harmony::Harmony>,
        state: &PictureState,
    ) -> Placed {
        use harmonigraph_core::tuning::PitchClass;
        let (tuning, tempered) = (state.runtime.tuning, state.runtime.tempered);
        let reach = state.appearance.view.reach();
        let kept = ctx
            .data(|d| d.get_temp::<Placed>(id))
            .filter(|kept| (kept.tuning, kept.tempered, kept.reach) == (tuning, tempered, reach));
        if let Some(kept) = &kept {
            if std::sync::Weak::as_ptr(&kept.harmony) == std::sync::Arc::as_ptr(harmony) {
                return kept.clone();
            }
        }
        let mut classes: Vec<u32> = harmony
            .sections
            .iter()
            .flat_map(|s| &s.entries)
            .map(|entry| PitchClass::from_cents(entry.pitch * 100.0).microcents())
            .collect();
        classes.sort_unstable();
        classes.dedup();
        let weak = std::sync::Arc::downgrade(harmony);
        if let Some(mut kept) = kept.filter(|kept| kept.classes == classes) {
            kept.harmony = weak;
            ctx.data_mut(|d| d.insert_temp(id, kept.clone()));
            return kept;
        }
        let nodes: Vec<(harmonigraph_core::LatticePos, PitchClass)> =
            reach.positions().map(|p| (p, tuning.pitch_class(p))).collect();
        let lit: std::collections::BTreeMap<u32, Option<harmonigraph_core::LatticePos>> = classes
            .iter()
            .map(|&class| {
                let pc = PitchClass::from_microcents(i64::from(class));
                (
                    class,
                    super::names::naming_node_from(nodes.iter().copied(), tempered, &tuning, pc),
                )
            })
            .collect();
        let mut frame = None::<Rect>;
        for at in lit.values().flatten().map(|&p| plan(p).to_pos2()) {
            let at = Rect::from_min_max(at, at);
            frame = Some(frame.map_or(at, |frame| frame.union(at)));
        }
        let placed = Placed {
            harmony: weak,
            classes,
            tuning,
            tempered,
            reach,
            lit: std::sync::Arc::new(lit),
            frame,
        };
        ctx.data_mut(|d| d.insert_temp(id, placed.clone()));
        placed
    }
}

/// The recording's harmony along a row over the overview (Tape mode, with the
/// far region open; #1974): one small lattice per section, on the overview's
/// own song axis, lighting the nodes that sound in it. A node is a set of
/// rings, one per octave it sounds in, lowest in the middle, each in its
/// note's colour, the one struck last where two share a ring. A pitch lights the node the roll names it by
/// ([`naming_node_from`](super::names::naming_node_from)): the one position of
/// the naming reach that matches it under the live tuning, and in a
/// temperament, where a dozen do, the one that spells most plainly. Every
/// match, as the lattice lights them, would make each thumbnail a wallpaper of
/// its chord. A pitch the reach does not hold lights none.
///
/// One frame holds every thumbnail, the nodes the whole recording lights, so a
/// harmony that moves across the lattice is a lit shape that moves in its
/// frame. Sections too narrow to hold that frame join the one before
/// ([`Harmony::cells`](crate::tape::harmony::Harmony::cells)).
///
/// Under a playhead that advances, its thumbnail fills in as the song does: a
/// note struck in the section appears at its attack and settles from white.
/// Nothing else tells a struck note from one held over, so a still row shows
/// each section whole. A click cues the audition from the section's start.
pub(super) fn harmony(ui: &mut egui::Ui, rect: Rect, state: &mut PictureState, surface: usize) {
    use harmonigraph_core::tuning::PitchClass;

    let cfg = state.appearance.spectrum;
    let axes = overview_axes(rect, &cfg);
    let window = Window::fit(&state.runtime.tape);
    let axis = SongAxis::new(&axes, 0.0, window);
    let id = egui::Id::unique(("tape-harmony", surface));
    let response = ui.interact(rect, id, egui::Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::picture());
    let x_of = |seconds: f64| axes.across_pitch(axis.depth_of(seconds))[0].x;

    let harmony = std::sync::Arc::clone(&state.runtime.tape.shown_harmony);
    let tuning = state.runtime.tuning;
    let class_of = |pitch: f32| PitchClass::from_cents(pitch * 100.0);
    let placed = Placed::of(ui.ctx(), id.with("placed"), &harmony, state);
    let (lit, frame) = (&placed.lit, placed.frame);
    let view = &state.runtime.tape;
    let (position_rate, heard_rate) = rates(view);
    // The playheads, over whatever the row drew.
    let playheads = |state: &mut PictureState| {
        if let Some(pixel) = marks(&painter, &axes, &state.runtime.tape, &axis, id.with("marks")) {
            state.surfaces.moving_this_frame.hold_for(crate::Reason::Playhead, pixel);
        }
    };
    // With nothing lit, a frame one node across: the sections still show,
    // and still cue.
    let frame = frame.unwrap_or(Rect::ZERO);

    let pad = 3.0 * theme::ui_scale(ui.ctx());
    let (columns, rows) = (frame.width() + 1.0, frame.height() + 1.0);
    let node = ((rect.height() - 2.0 * pad) / rows)
        .min((rect.width() - 2.0 * pad) / columns)
        .min(NODE_SHARE * rect.height());
    let thumbnail = egui::vec2(columns, rows) * node;
    let narrowest = f64::from(thumbnail.x + 2.0 * pad) * axis.seconds_per_point();
    let cells = harmony.cells(narrowest, window.to);

    // The playhead a thumbnail fills in under: the audition's, or the song's
    // while the transport rolls. Still advancing is the playheads' own test
    // (`marks`): its value changed within the stall allowance.
    let under = view.heard.map(|at| (at, heard_rate)).or(view.position.zip(position_rate));
    let clock = ui.input(|i| i.time);
    // A cue's first frame already hides what the section has not yet struck.
    let moving =
        ui.data_mut(|d| advancing(d.get_temp_mut_or_default(id), under.map(|(at, _)| at), clock));
    let mut next_change = None::<f64>;

    let white = egui::Color32::WHITE;
    let ruling = theme::picture_ruling();
    let mut cue = None;
    let pointer = response.hover_pos();
    for (k, cell) in cells.iter().enumerate() {
        let next = cells.get(k + 1).map_or(f64::INFINITY, |next| next.start);
        // At least a thumbnail wide, which the next start, or the row's end,
        // leaves room for.
        let (a, b) = (x_of(cell.start), x_of(cell.end.max(cell.start + narrowest)));
        let span = Rect::from_x_y_ranges(a.min(b)..=a.max(b), rect.y_range());
        if k % 2 == 1 {
            painter.rect_filled(span.intersect(rect), 0.0, theme::header().gamma_multiply(0.35));
        }
        painter.line_segment(
            [egui::pos2(a, rect.top()), egui::pos2(a, rect.bottom())],
            Stroke::new(theme::GRID_WIDTH, theme::picture_divider()),
        );
        if pointer.is_some_and(|p| span.contains(p)) {
            cue = Some(cell.start);
        }
        let origin = span.center() - 0.5 * thumbnail - frame.min.to_vec2() * node
            + egui::vec2(0.5 * node, 0.5 * node);
        let centre = |pos: harmonigraph_core::LatticePos| origin + plan(pos) * node;
        for fives in frame.min.x.ceil() as i32..=frame.max.x.floor() as i32 {
            for threes in (-frame.max.y).ceil() as i32..=(-frame.min.y).floor() as i32 {
                let at = centre(harmonigraph_core::LatticePos::new(threes, fives, 0));
                painter.circle_filled(at, (0.06 * node).max(0.5), ruling);
            }
        }
        // Under an advancing playhead, only what has been struck by now.
        let now = under.filter(|&(at, _)| moving && (cell.start..next).contains(&at));
        // A node's rings: each the octave it sounds in, how far its flash has
        // to settle, and its colour.
        type Ring = (i32, f32, egui::Color32);
        let mut nodes: Vec<(harmonigraph_core::LatticePos, Vec<Ring>)> = Vec::new();
        let mut soonest = |wait: f64| next_change = Some(next_change.map_or(wait, |s| s.min(wait)));
        for entry in &cell.entries {
            let mut flash = 0.0;
            if let Some((at, rate)) = now {
                // Each strike is a frame at the time it sounds.
                if let Some(&next) = entry.strikes.iter().find(|&&struck| struck > at) {
                    soonest((next - at) / rate);
                }
                if !entry.held && entry.strikes.first().is_none_or(|&first| first > at) {
                    continue;
                }
                // The last strike by now flashes, settling at the chrome's
                // rate.
                let age = entry.strikes.iter().rev().find(|&&struck| struck <= at).map(|s| at - s);
                if let Some(age) = age.filter(|&age| age < FLASH_SETTLED * FLASH_SECONDS) {
                    flash = (-age / FLASH_SECONDS).exp() as f32;
                    soonest(crate::CHROME_CUE);
                }
            }
            let colour = roll::note_color(state, entry.pitch, 1.0);
            let colour = white.lerp_to_gamma(colour, 1.0 - flash);
            for &pos in lit.get(&class_of(entry.pitch).microcents()).into_iter().flatten() {
                // Octaves counted from the node's own pitch, which a pitch
                // that lights it is within the tolerance of.
                let above = entry.pitch * 100.0 - tuning.pitch_class(pos).to_cents();
                let octave = (above / 1200.0).round() as i32;
                match nodes.iter_mut().find(|(at, _)| *at == pos) {
                    Some((_, rings)) => match rings.iter_mut().find(|ring| ring.0 == octave) {
                        // Two notes on one ring: the one struck last shows.
                        Some(ring) if flash > ring.1 => *ring = (octave, flash, colour),
                        Some(_) => {}
                        None => rings.push((octave, flash, colour)),
                    },
                    None => nodes.push((pos, vec![(octave, flash, colour)])),
                }
            }
        }
        for (pos, rings) in &nodes {
            // Outermost first, so each lower pitch is drawn over the one above.
            for (ring, &(_, _, colour)) in rings.iter().enumerate().rev() {
                let radius = 0.44 * node * (ring + 1) as f32 / rings.len() as f32;
                painter.circle_filled(centre(*pos), radius, colour);
                if ring + 1 < rings.len() {
                    painter.circle_stroke(
                        centre(*pos),
                        radius,
                        Stroke::new(theme::GRID_WIDTH, theme::picture()),
                    );
                }
            }
        }
    }
    if let Some(wait) = next_change {
        state.surfaces.moving_this_frame.hold_for(crate::Reason::Playhead, wait);
    }
    playheads(state);
    let view = &mut state.runtime.tape;
    if let Some(start) = cue {
        if view.enabled && !view.rolling {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if response.clicked() {
                view.request = Some(TapeRequest::Cue(start));
            }
        }
        response.on_hover_text_at_pointer(place(view, start));
    }
}

/// How near an end of the shown stretch, in points, a press takes hold of
/// that end rather than of the whole stretch.
const GRIP_PT: f32 = 5.0;

/// What an overview drag holds: the shown stretch, or one of its ends. Which
/// one is decided by where the press landed and kept for the whole drag.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grip {
    Move,
    From,
    To,
}

/// An overview drag under way: what it holds, the song seconds it was pressed
/// at, and the stretch as it was then. Each frame places the stretch from
/// these, so a drag pushed past the song's start comes back to the hand.
#[derive(Clone, Copy, Debug)]
struct Held {
    grip: Grip,
    pressed: f64,
    start: Window,
}

/// The overview's axes: song time across the strip, pitch up it. The song
/// starts on the side the far region starts it where time runs across the
/// pane, and on the left where it runs down it.
pub(super) fn overview_axes(rect: Rect, cfg: &crate::SpectrumConfig) -> Axes {
    let pane = Axes::new(rect, cfg);
    Axes { rect, time_vertical: false, time_reversed: pane.time_vertical || pane.time_reversed }
}

/// The whole recording along a strip under the pane (Tape mode, with the far
/// region open), with the stretch the far region shows boxed on it. The box
/// is moved by dragging it and resized by dragging an end; a click outside it
/// moves it there, and a double-click shows the whole recording again.
///
/// Drawn before the pane, so a stretch moved here is the one the pane draws
/// this frame.
pub(super) fn overview(ui: &mut egui::Ui, rect: Rect, state: &mut PictureState, surface: usize) {
    let cfg = state.appearance.spectrum;
    let axes = overview_axes(rect, &cfg);
    let whole = Window::fit(&state.runtime.tape);
    let axis = SongAxis::new(&axes, 0.0, whole);
    let id = egui::Id::unique(("tape-overview", surface));
    let response = ui.interact(rect, id, egui::Sense::click_and_drag());
    let shown = window(ui, &state.runtime.tape, surface);
    // Held to the strip: a drag carried past its end would otherwise take the
    // box off the strip, where nothing shows it or can take hold of it.
    let seconds_at =
        |x: f32| axis.seconds_at(axes.depth_at(Pos2::new(x, rect.center().y))).clamp(0.0, whole.to);
    let grip_at = |x: f32| grip(&axes, &axis, shown, x);

    let mut moved = None;
    if response.double_clicked() {
        moved = Some(whole);
    } else if response.drag_started() {
        if let Some(press) = ui.input(|i| i.pointer.press_origin()) {
            let pressed = seconds_at(press.x);
            // A press off the box brings it there, and the drag carries it on.
            let (grip, start) = match grip_at(press.x) {
                Some(grip) => (grip, shown),
                None => (Grip::Move, shown.centred(pressed)),
            };
            ui.data_mut(|d| d.insert_temp(id, Held { grip, pressed, start }));
            moved = Some(start);
        }
    } else if response.clicked() {
        let at = response.interact_pointer_pos().map(|p| p.x);
        if let Some(x) = at.filter(|&x| grip_at(x).is_none()) {
            moved = Some(shown.centred(seconds_at(x)));
        }
    }
    let held = ui.data(|d| d.get_temp::<Held>(id));
    if let (true, Some(held), Some(at)) =
        (response.dragged(), held, response.interact_pointer_pos())
    {
        let at = seconds_at(at.x);
        // On the strip as it is now, which grows while recording.
        let start = Window { end: whole.end, ..held.start };
        moved = Some(match held.grip {
            Grip::Move => start.moved(at - held.pressed),
            Grip::From => start.with_from(at),
            Grip::To => start.with_to(at),
        });
    }
    if response.drag_stopped() {
        ui.data_mut(|d| d.remove::<Held>(id));
    }
    let shown = moved.unwrap_or(shown);
    if let Some(window) = moved {
        ui.data_mut(|d| d.insert_temp(window_id(surface), window));
    }
    let cursor = match held.filter(|_| response.dragged()).map(|held| held.grip) {
        Some(Grip::Move) => Some(egui::CursorIcon::Grabbing),
        Some(_) => Some(egui::CursorIcon::ResizeHorizontal),
        None => response.hover_pos().and_then(|p| grip_at(p.x)).map(|grip| match grip {
            Grip::Move => egui::CursorIcon::Grab,
            _ => egui::CursorIcon::ResizeHorizontal,
        }),
    };
    if let Some(cursor) = cursor {
        ui.ctx().set_cursor_icon(cursor);
    }

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, crate::spectrogram::silence_color(cfg.spectrogram_gradient));
    let scale = PitchScale::of(&cfg);
    heatmap(&painter, &axes, &scale, state, &axis, surface, TapeStrip::Overview);
    let view = &state.runtime.tape;
    coverage(&painter, &axes, view, &axis);
    if let Some(pixel) = marks(&painter, &axes, view, &axis, id.with("marks")) {
        state.surfaces.moving_this_frame.hold_for(crate::Reason::Playhead, pixel);
    }
    let view = &state.runtime.tape;
    // The rest of the song dimmed, and the stretch shown outlined.
    let (a, b) = span_x(&axes, &axis, shown);
    let (lo, hi) = (a.min(b).max(rect.left()), a.max(b).min(rect.right()));
    let dim = theme::picture().gamma_multiply(0.55);
    painter.rect_filled(Rect::from_x_y_ranges(rect.left()..=lo, rect.y_range()), 0.0, dim);
    painter.rect_filled(Rect::from_x_y_ranges(hi..=rect.right(), rect.y_range()), 0.0, dim);
    painter.rect_stroke(
        Rect::from_x_y_ranges(lo..=hi.max(lo + 1.0), rect.y_range()).shrink(0.5),
        0.0,
        Stroke::new(theme::LINE_WIDTH, theme::accent()),
        egui::StrokeKind::Inside,
    );
    if let Some(pointer) = response.hover_pos() {
        let at = seconds_at(pointer.x).max(0.0);
        response.on_hover_text_at_pointer(describe(view, at));
    }
}

/// Where the stretch `shown` lies across the overview: its screen x at its
/// start and at its end.
fn span_x(axes: &Axes, axis: &SongAxis, shown: Window) -> (f32, f32) {
    let x = |seconds| axes.at(0.0, axis.depth_of(seconds)).x;
    (x(shown.from), x(shown.to))
}

/// What a press at `x` on the overview takes hold of: inside the box, the
/// whole stretch, or an end within [`GRIP_PT`] of it; just outside the box,
/// the end beside it. The ends only reach inside a box wide enough to leave
/// it a middle to move by.
fn grip(axes: &Axes, axis: &SongAxis, shown: Window, x: f32) -> Option<Grip> {
    let (a, b) = span_x(axes, axis, shown);
    let ((lo, lo_grip), (hi, hi_grip)) =
        if a <= b { ((a, Grip::From), (b, Grip::To)) } else { ((b, Grip::To), (a, Grip::From)) };
    let inner = if hi - lo >= 3.0 * GRIP_PT { GRIP_PT } else { 0.0 };
    if x >= lo - GRIP_PT && x < lo + inner {
        Some(lo_grip)
    } else if x > hi - inner && x <= hi + GRIP_PT {
        Some(hi_grip)
    } else if (lo..=hi).contains(&x) {
        Some(Grip::Move)
    } else {
        None
    }
}

/// The controls' one line (#1719 slice 4): why the tape cannot sound, or what
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
    fn the_line_says_why_the_tape_is_silent_or_what_is_recorded_where_it_plays() {
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
    fn the_fit_follows_the_position_only_while_it_rolls() {
        let span = crate::tape::TapeSpan {
            start: 0.0,
            end: 40.0,
            warm: true,
            recorded: std::time::Instant::now(),
        };
        let mut view = TapeView { spans: vec![span], position: Some(600.0), ..Default::default() };
        assert_eq!(Window::reach(&view, 0.0), 40.0, "stopped far past the recording");
        view.rolling = true;
        assert_eq!(Window::reach(&view, 0.0), 600.0, "rolling there");
    }

    #[test]
    fn zooming_out_stops_at_the_whole_strip() {
        let mut window = Window { from: 20.0, to: 30.0, zoomed: true, end: 60.0 };
        for _ in 0..10 {
            window.zoom(0.5, 25.0);
        }
        assert_eq!((window.from, window.to), (0.0, 60.0));
    }

    /// A pan or a zoom stops at the strip's end with its width kept, as at
    /// the song's start (#1722's scroll pan had no end), and one that cannot
    /// move leaves a window following the recording.
    #[test]
    fn a_move_stops_at_the_end_of_the_strip_with_its_width_kept() {
        let whole = Window { from: 0.0, to: 42.0, zoomed: false, end: 42.0 };
        assert_eq!(whole.moved(100.0), whole, "a move that cannot move pinned the window");
        let mut window = whole;
        window.zoom(4.0, 21.0);
        let width = window.seconds();
        let mut window = window.moved(1000.0);
        assert_eq!((window.from, window.to), (42.0 - width, 42.0));
        // Zoomed out about a point near the end, whose share would carry the
        // wider stretch past it.
        window.zoom(0.5, 41.0);
        assert_eq!((window.from, window.to), (42.0 - 2.0 * width, 42.0));
    }
}
