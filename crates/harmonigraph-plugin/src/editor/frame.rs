use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use egui_baseview::Queue;
use nice_plug::prelude::{GuiContext, ParamSetter};

use crate::{HarmonigraphParams, PluginParamBackend};

use super::lock::EditorLock;
use super::persist::EguiState;
use super::pop_out::OwnWindow;

/// The plugin's per-frame GUI work: take the frame's lock, close out the note
/// frame — and then hand the state the input worker has fed to
/// [`harmonigraph_ui::shell::Frame`], which draws it and answers with
/// whatever the folds it settled leave the window needing, with the take
/// reconciled around it ([`EditorShared::take_pass`](super::EditorShared::take_pass)).
///
/// What stays here is what only this shell can do: the take and the params
/// are its own, and so is the frame rate at the end, since it is the only
/// shell whose window takes one. The standalone harness's counterpart is
/// `App::ui` in harmonigraph-standalone, and the sequence the two share is one
/// call each rather than a list each keeps in step by hand.
pub(super) fn frame(
    ui: &mut egui::Ui,
    queue: &mut Queue,
    state: &mut WindowState,
    egui_state: &EguiState,
    context: &dyn GuiContext,
) {
    // The view does not own the native controller; closing its editor handle
    // must be able to release the view and its callbacks.
    let Some(pop_out) = state.pop_out.upgrade() else { return };

    // Everything the shell does before the UI runs: the lock, a restore and
    // the camera. Timed separately because `ui cpu` starts at the dock build,
    // so this stretch sits outside every other CPU reading; the last pass's
    // take work, around its UI, is added to it below.
    // The rings and their FFTs are the input worker's, off this thread.
    let shell_start = Instant::now();
    let setter = ParamSetter::new(context);

    // One lock for the whole frame, which also installs a host restore made
    // since the last holder (a preset load, an undo; see `EditorLock`). The
    // audio thread only ever touches the rtrb producers, and the editor-drop
    // path runs on this same GUI thread. The other parties hold it briefly:
    // the input worker, for its take and its give but never for its FFTs, and
    // a host state save (`UiState`'s `map`), for one serialization on the
    // host's main thread, which is this thread.
    let mut guard = state.shared.lock();
    let shared = &mut *guard;

    // The display link's timing for this frame, or `None` for one it did not ask
    // for (the reveal's, run from the occlusion notification).
    let timing = queue.frame_timing();
    // A stalled run loop (run-loop mode issues, a host blocking its main
    // thread), so freezes stay attributable: measured by the window on every
    // display frame, including the ones that ran no pass.
    if let Some(gap) = queue.longest_tick_gap().filter(|gap| gap.as_secs_f64() > STALL_GAP) {
        let ms = gap.as_secs_f64() * 1000.0;
        shared.ui.picture.runtime.console.log(format!("frame stall: {ms:.0} ms between updates"));
    }

    // The input worker ([`crate::background`]) drains the rings, runs the
    // analysis and polls the take's end; a frame reads what it left.
    let now = shared.input.now();
    // Drained on the clock it arrived by, drawn at the refresh it is for.
    let wall = shared.input.frame_wall(timing, now);
    let now = shared.input.display_now(wall);
    let sample_rate = shared.input.sample_rate();
    if let Some(line) = state.params.settings.take_notice() {
        shared.ui.log(line);
    }
    // Snapshot current host movement before arming or queuing a render.
    let configuration = state.params.settings.view();
    shared.ui.picture.appearance.sync_camera(&PluginParamBackend {
        params: &state.params,
        setter: &setter,
        gestures: &shared.gestures,
        configuration,
    });
    // Last frame's costs that the shell measures and the UI cannot: they
    // happen after `root_ui` returns.
    //
    // One literal rather than a field at a time, so a reading the window
    // starts publishing and this forgets to copy is a missing field the
    // compiler names, not a line nobody misses — the overlay would otherwise
    // show it as a steady zero, which reads as a cost that isn't there.
    // `shell_ms` comes last because it measures everything above it, and
    // carries the last pass's take work, which ran after its reading.
    shared.ui.picture.instruments.timings = harmonigraph_perf::ShellTimings {
        tess_ms: queue.tess_ms(),
        draw_gpu_ms: queue.take_draw_gpu_ms(),
        acquire_ms: queue.acquire_ms(),
        tick_ms: queue.tick_ms(),
        render_ms: queue.render_ms(),
        upload_ms: queue.upload_ms(),
        ubuf_ms: queue.ubuf_ms(),
        texture_ms: queue.texture_ms(),
        prims: queue.prims(),
        verts: queue.verts(),
        encode_ms: queue.encode_ms(),
        submit_ms: queue.submit_ms(),
        target_fps: timing
            .and_then(|timing| harmonigraph_perf::link_target_fps(timing.refresh, timing.divisor)),
        shell_ms: shell_start.elapsed().as_secs_f32() * 1000.0 + state.take_ms,
    };
    // A fold is priced against `egui_state.size`, which is the size the window
    // actually has: `requested_size` is empty by now whatever happened above,
    // since the window's size source clears it on refusal as well as on
    // accept, and `size` is only updated when the host agrees. So two folds in
    // consecutive frames add up rather than the second cancelling the first.
    let (window_width, window_height) = egui_state.size();
    // Offered only where the shell can give the editor its own window, and
    // read back after the UI has had the chance to flip it.
    shared.ui.workspace.interaction.own_window = pop_out.offered().then(|| !egui_state.docked());
    let take_start = Instant::now();
    let mut drawing = Duration::ZERO;
    let ask = shared.take_pass(sample_rate, |shared| {
        let backend = PluginParamBackend {
            params: &state.params,
            setter: &setter,
            gestures: &shared.gestures,
            configuration,
        };
        let drawn = Instant::now();
        let ask = harmonigraph_ui::shell::Frame {
            ui,
            state: &mut shared.ui,
            params: &backend,
            now,
            build_tag: crate::BUILD_TAG,
            window_size: egui::vec2(window_width as f32, window_height as f32),
        }
        .draw();
        drawing = drawn.elapsed();
        ask
    });
    // The take's mirror before the UI and its commands after (a take's start
    // makes its directory and hands the writer its channel): main-thread time
    // in no other reading, so the next frame's shell reading carries it.
    state.take_ms = take_start.elapsed().saturating_sub(drawing).as_secs_f32() * 1000.0;
    #[cfg(all(feature = "frame-log", target_os = "macos"))]
    state.logged.note(&shared.ui);

    // Also where a host restore that moved the setting reaches an open editor.
    if let Some(own) = shared.ui.workspace.interaction.own_window {
        egui_state.set_docked(!own);
        pop_out.want(own);
    }

    // A pane folded sideways (or came back) leaves every other pane its width
    // and asks the WINDOW for the difference. Queued rather than applied:
    // `request_resize` is the host's round trip, and the window's size source
    // — which runs at the top of the next tick, before the frame it will be
    // laid out in — is where this window takes it (see `LatticeEditor::spawn`).
    if let Some(size) = ask {
        egui_state.requested_size.store(Some((size.x.round() as u32, size.y.round() as u32)));
        ui.ctx().request_repaint();
    }

    // AFTER the UI has run, so a rate picked this frame takes effect on the
    // next refresh rather than the one after it.
    queue.set_frame_divisor(shared.ui.workspace.interaction.prefs.frame_rate.divisor());
    // What this frame claimed, the editor's until the next frame says
    // otherwise: a key arrives between frames (see `harmonigraph_ui::keys`).
    // A focused text field's own keys, and the shell's own Escape and Cmd-C
    // claims (a drag, a popup, a selected label), are the editor's too; every
    // other key goes to the host.
    queue.set_shortcuts(&harmonigraph_ui::keys::claimed(ui.ctx()));
}

/// The gap between display frames, in seconds, past which a pass logs a stall:
/// clear of any interval the link paces at, Quarter at 60 Hz being 67 ms.
const STALL_GAP: f64 = 0.1;

/// State handed to egui-baseview's run loop.
pub(super) struct WindowState {
    pub(super) shared: Arc<EditorLock>,
    pub(super) params: Arc<HarmonigraphParams>,
    pub(super) pop_out: Weak<OwnWindow>,
    /// The last pass's take work, in ms, for this pass's shell reading.
    take_ms: f32,
    /// What the frame log was last told the editor was doing.
    #[cfg(all(feature = "frame-log", target_os = "macos"))]
    logged: Logged,
}

impl WindowState {
    pub(super) fn new(
        shared: Arc<EditorLock>,
        params: Arc<HarmonigraphParams>,
        pop_out: Weak<OwnWindow>,
    ) -> Self {
        WindowState {
            shared,
            params,
            pop_out,
            take_ms: 0.0,
            #[cfg(all(feature = "frame-log", target_os = "macos"))]
            logged: Logged::default(),
        }
    }
}

/// The editor's side of the frame log: a row as the window opens, with the
/// Span, and one each time why it animates or how many exports run changes,
/// which `tools/frame-log-summary.py --events` reads to find the silent
/// stretches, first notes and exports #1670 asks about. `None` until the
/// window's first frame, so that frame logs all three.
#[cfg(all(feature = "frame-log", target_os = "macos"))]
#[derive(Default)]
struct Logged {
    animating: Option<harmonigraph_ui::Animating>,
    exports: Option<usize>,
}

#[cfg(all(feature = "frame-log", target_os = "macos"))]
impl Logged {
    fn note(&mut self, ui: &harmonigraph_ui::SharedState) {
        if self.animating.is_none() {
            let span = ui.picture.appearance.spectrum.roll_seconds;
            egui_baseview::frame_log::event(&format!("open span={span}"));
        }
        let animating = harmonigraph_ui::Animating::of(ui);
        if self.animating != Some(animating) {
            self.animating = Some(animating);
            egui_baseview::frame_log::event(&format!("animating {animating}"));
        }
        // A cancelled job's renderer runs until it is reaped, so it counts.
        // No encoder: the pane's says nothing about what an automatic export
        // or an over-size frame ran through.
        use harmonigraph_take::render::ExportStatus::{Cancelling, Running};
        let exports = ui.workspace.interaction.take.exports.iter();
        let running = exports.filter(|job| matches!(job.state, Running | Cancelling)).count();
        if self.exports != Some(running) {
            self.exports = Some(running);
            egui_baseview::frame_log::event(&format!("exports {running}"));
        }
    }
}
