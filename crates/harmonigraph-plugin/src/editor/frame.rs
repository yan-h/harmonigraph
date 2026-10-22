use std::sync::Arc;
use std::time::Instant;

use egui_baseview::Queue;
use nice_plug::prelude::{GuiContext, ParamSetter};
use parking_lot::Mutex;

use crate::{HarmonigraphParams, PluginParamBackend};

use super::persist::EguiState;
use super::shared::EditorShared;

/// The plugin's per-frame GUI work: take the frame's lock, close out the note
/// frame, drain the MIDI and audio rings, reconcile the take — and then hand
/// the state it has just fed to [`harmonigraph_ui::shell::Frame`], which draws
/// it and answers with whatever the folds it settled leave the window needing.
///
/// What stays here is what only this shell can do: the rings are its own, and
/// so is the frame rate at the end, since it is the only shell whose window
/// takes one. The standalone harness's counterpart is
/// `App::ui` in harmonigraph-standalone, and the sequence the two share is one
/// call each rather than a list each keeps in step by hand.
pub(super) fn frame(
    ui: &mut egui::Ui,
    queue: &mut Queue,
    state: &mut WindowState,
    egui_state: &EguiState,
    context: &dyn GuiContext,
) {
    // Everything the shell does before the UI runs: draining the MIDI and
    // audio rings and reconciling the take. Timed separately because `ui cpu`
    // starts at the dock build, so this whole stretch — which scales with the
    // number of events arriving, i.e. exactly with how hard you are playing —
    // sits outside every other CPU reading.
    let shell_start = Instant::now();
    let setter = ParamSetter::new(context);

    // One lock for the whole frame. Uncontended by design: the audio
    // thread only ever touches the rtrb producer, and the editor-drop
    // path runs on this same GUI thread. The one other party is a host
    // state save with the window open (`UiState`'s `map`), which takes it
    // for one serialization: on the host's main thread, which is this
    // thread on macOS and Windows and a different one on X11.
    let mut guard = state.shared.lock();
    let shared = &mut *guard;

    // A host restore since the last frame (a preset load, an undo), which
    // nothing else applies to an open window. Before the drain, for the reason
    // the background analyzer adopts one before its own: the settings decide
    // how the samples about to be taken are analyzed.
    state.params.ui_state.apply_restore(&mut shared.ui);

    shared.note_frame();

    let now = shared.input.now();
    if shared.input.drain(&mut shared.ui.picture.runtime, &shared.ui.picture.appearance, now) {
        // New MIDI must render this tick, not at the idle poll.
        ui.ctx().request_repaint();
    }
    let now = shared.input.display_now(now);
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
        shell_ms: shell_start.elapsed().as_secs_f32() * 1000.0,
    };
    // A fold is priced against `egui_state.size`, which is the size the window
    // actually has: `requested_size` is empty by now whatever happened above,
    // since the window's size source clears it on refusal as well as on
    // accept, and `size` is only updated when the host agrees. So two folds in
    // consecutive frames add up rather than the second cancelling the first.
    let (window_width, window_height) = egui_state.size();
    let ask = harmonigraph_ui::shell::Frame {
        ui,
        state: &mut shared.ui,
        params: &backend,
        now,
        build_tag: crate::BUILD_TAG,
        window_size: egui::vec2(window_width as f32, window_height as f32),
    }
    .draw();

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
}

/// State handed to egui-baseview's run loop.
pub(super) struct WindowState {
    pub(super) shared: Arc<Mutex<EditorShared>>,
    pub(super) params: Arc<HarmonigraphParams>,
}

impl WindowState {
    pub(super) fn new(shared: Arc<Mutex<EditorShared>>, params: Arc<HarmonigraphParams>) -> Self {
        WindowState { shared, params }
    }
}
