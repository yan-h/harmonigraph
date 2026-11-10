use std::sync::{Arc, Weak};
use std::time::Instant;

use egui_baseview::Queue;
use nice_plug::prelude::{GuiContext, ParamSetter};
use parking_lot::Mutex;

use crate::{HarmonigraphParams, PluginParamBackend};

use super::persist::EguiState;
use super::pop_out::PopOut;
use super::shared::EditorShared;

/// The plugin's per-frame GUI work: take the frame's lock, close out the note
/// frame, reconcile the take — and then hand the state the input worker has
/// fed to [`harmonigraph_ui::shell::Frame`], which draws it and answers with
/// whatever the folds it settled leave the window needing.
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

    // Everything the shell does before the UI runs: the lock, a restore,
    // the camera and the take. Timed separately because `ui cpu` starts at
    // the dock build, so this stretch sits outside every other CPU reading.
    // The rings and their FFTs are the input worker's, off this thread.
    let shell_start = Instant::now();
    let setter = ParamSetter::new(context);

    // One lock for the whole frame. The audio thread only ever touches the
    // rtrb producers, and the editor-drop path runs on this same GUI thread.
    // The other parties hold it briefly: the input worker, for its take and
    // its give but never for its FFTs, and a host state save with the window
    // open (`UiState`'s `map`), for one serialization on the host's main
    // thread, which is this thread on macOS and Windows and a different one
    // on X11.
    let mut guard = state.shared.lock();
    let shared = &mut *guard;

    // A host restore since the last frame (a preset load, an undo), which
    // nothing else applies to an open window. The input worker's next take
    // reads the settings it leaves, which decide how the samples taken then
    // are analyzed.
    state.params.ui_state.apply_restore(&mut shared.ui);

    // The display link's timing for this frame, or `None` for one it did not ask
    // for (the watchdog's, an occlusion repaint).
    let timing = queue.frame_timing();
    shared.note_frame(timing.is_some() || !cfg!(target_os = "macos"));

    // The input worker ([`crate::background`]) drains the rings and runs the
    // analysis, and a frame reads what it left — unless it never started.
    let now = shared.input.now();
    if shared.frame_drains {
        let runtime = &mut shared.ui.picture.runtime;
        shared.input.drain(runtime, &shared.ui.picture.appearance, now);
    }
    // Drained on the clock it arrived by, drawn at the refresh it is for.
    let wall = shared.input.frame_wall(timing, now);
    let now = shared.input.display_now(wall);
    let sample_rate = shared.input.sample_rate();
    // Snapshot current host movement before arming or queuing a render.
    shared.ui.picture.appearance.sync_camera(&PluginParamBackend {
        params: &state.params,
        setter: &setter,
        gesture: &shared.gesture,
        configuration: None,
    });
    shared.sync_take(sample_rate);

    let backend = PluginParamBackend {
        params: &state.params,
        setter: &setter,
        gesture: &shared.gesture,
        configuration: state.params.configuration.get().map(|mailbox| mailbox.visible()),
    };
    // Last frame's costs that the shell measures and the UI cannot: they
    // happen after `root_ui` returns.
    //
    // One literal rather than a field at a time, so a reading the window
    // starts publishing and this forgets to copy is a missing field the
    // compiler names, not a line nobody misses — the overlay would otherwise
    // show it as a steady zero, which reads as a cost that isn't there.
    // `shell_ms` comes last because it measures everything above it.
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
        shell_ms: shell_start.elapsed().as_secs_f32() * 1000.0,
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
    let ask = harmonigraph_ui::shell::Frame {
        ui,
        state: &mut shared.ui,
        params: &backend,
        now,
        build_tag: crate::BUILD_TAG,
        window_size: egui::vec2(window_width as f32, window_height as f32),
    }
    .draw();
    #[cfg(all(feature = "frame-log", target_os = "macos"))]
    state.logged.note(&shared.ui, now);

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
    queue.set_frame_divisor(shared.ui.workspace.interaction.frame_rate.divisor());
    queue.set_focus_takes_only_arrows(harmonigraph_ui::plot_handle_focused(ui.ctx()));
    queue.set_key_capture(key_capture(shared.ui.picture.runtime.map_arrows));
}

/// The keys the editor keeps from the host even with nothing focused: the
/// appearance undo and redo, the chrome toggle, and while the lattice moves a
/// map under the pointer, the arrows it takes.
pub(super) fn key_capture(map_arrows: bool) -> egui_baseview::KeyCapture {
    let mut shortcuts = harmonigraph_ui::APPEARANCE_SHORTCUTS.to_vec();
    shortcuts.push(harmonigraph_ui::FRAMELESS_SHORTCUT);
    if map_arrows {
        shortcuts.extend(harmonigraph_ui::MAP_ARROW_SHORTCUTS);
    }
    egui_baseview::KeyCapture::CaptureShortcuts(shortcuts)
}

/// State handed to egui-baseview's run loop.
pub(super) struct WindowState {
    pub(super) shared: Arc<Mutex<EditorShared>>,
    pub(super) params: Arc<HarmonigraphParams>,
    pub(super) pop_out: Weak<PopOut>,
    /// What the frame log was last told the editor was doing.
    #[cfg(all(feature = "frame-log", target_os = "macos"))]
    logged: Logged,
}

impl WindowState {
    pub(super) fn new(
        shared: Arc<Mutex<EditorShared>>,
        params: Arc<HarmonigraphParams>,
        pop_out: Weak<PopOut>,
    ) -> Self {
        WindowState {
            shared,
            params,
            pop_out,
            #[cfg(all(feature = "frame-log", target_os = "macos"))]
            logged: Logged::default(),
        }
    }
}

/// The editor's side of the frame log: a row as the window opens, with the
/// Span, and one each time why it animates or the running export changes,
/// which `tools/frame-log-summary.py --events` reads to find the silent
/// stretches, first notes and exports #1670 asks about. `None` until the
/// window's first frame, so that frame logs all three.
#[cfg(all(feature = "frame-log", target_os = "macos"))]
#[derive(Default)]
struct Logged {
    animating: Option<harmonigraph_ui::Animating>,
    /// Everything the `exports` event prints: how many run, and the running
    /// job with the encoder its renderer reported. The count alone would log
    /// two exports back to back as one, since the next starts within a
    /// millisecond of the last, and never log a label arriving after its job.
    exports: Option<(usize, Option<RunningExport>)>,
}

/// The running export's job id, and the encoder its renderer reported.
#[cfg(all(feature = "frame-log", target_os = "macos"))]
type RunningExport = (u64, Option<harmonigraph_ui::VideoEncoder>);

#[cfg(all(feature = "frame-log", target_os = "macos"))]
impl Logged {
    fn note(&mut self, ui: &harmonigraph_ui::SharedState, now: f64) {
        if self.animating.is_none() {
            let span = ui.picture.appearance.spectrum.roll_seconds;
            egui_baseview::frame_log::event(&format!("open span={span}"));
        }
        let animating = harmonigraph_ui::Animating::of(ui, now);
        if self.animating != Some(animating) {
            self.animating = Some(animating);
            egui_baseview::frame_log::event(&format!("animating {animating}"));
        }
        let exports = &ui.workspace.interaction.take.exports;
        let is_running = |job: &&harmonigraph_take::render::ExportJob| {
            job.state == harmonigraph_take::render::ExportStatus::Running
        };
        let running = exports.iter().filter(is_running).count();
        let job = exports.iter().find(is_running).map(|job| (job.id, job.encoder));
        if self.exports != Some((running, job)) {
            self.exports = Some((running, job));
            // The encoder the renderer said it started, not the pane's: a frame
            // over the hardware limit runs x264, and an automatic export the
            // take's own look.
            let event = match job {
                Some((id, encoder)) => {
                    let encoder = encoder.map_or("starting", |encoder| encoder.name());
                    format!("exports {running} {encoder} job={id}")
                }
                None => format!("exports {running} none"),
            };
            egui_baseview::frame_log::event(&event);
        }
    }
}
