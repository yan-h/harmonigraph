use std::sync::Arc;
use std::time::Instant;

use egui_baseview::Queue;
use nice_plug::prelude::{GuiContext, ParamSetter};
use parking_lot::Mutex;

use crate::{HarmonigraphParams, PluginParamBackend};

use super::persist::EguiState;
use super::shared::EditorShared;

// Reached only through `tests` below, which builds an `EditorShared` to
// satisfy `WindowState`'s constructor.
#[cfg(test)]
use std::sync::atomic::AtomicU32;

/// The plugin's per-frame GUI work: take the frame's lock, close out the note
/// frame, drain the MIDI and audio rings, reconcile the take — and then hand
/// the state it has just fed to [`harmonigraph_ui::shell::Frame`], which draws
/// it and answers with whatever the folds it settled leave the window needing.
///
/// What stays here is what only this shell can do: the rings are its own, and
/// so is the pacing at the end, since it is the only shell asked for frames
/// faster than it means to draw them. The standalone harness's counterpart is
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
    // path runs on this same GUI thread.
    let mut guard = state.shared.lock();
    let shared = &mut *guard;

    shared.note_frame();

    let now = shared.input.now();
    if shared.input.drain(&mut shared.ui.picture.runtime, &shared.ui.picture.appearance, now) {
        // New MIDI must render this tick, not at the idle poll.
        ui.ctx().request_repaint();
    }
    let now = shared.input.display_now(now);
    let sample_rate = shared.input.sample_rate();
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
        egui_gpu_ms: queue.egui_gpu_ms(),
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
        window_width: window_width as f32,
    }
    .draw();

    // A pane folded sideways (or came back) leaves every other pane its width
    // and asks the WINDOW for the difference. Queued rather than applied:
    // `request_resize` is the host's round trip, and the window's size source
    // — which runs at the top of the next tick, before the frame it will be
    // laid out in — is where this window takes it (see `LatticeEditor::spawn`).
    if let Some(width) = ask {
        egui_state.requested_size.store(Some((width.round() as u32, window_height)));
        ui.ctx().request_repaint();
    }

    // Pace the window AFTER the UI has run, so a cap picked this frame takes
    // effect on the next tick rather than the one after it. The lock is
    // released first because what is armed is a fact about the WINDOW rather
    // than about the state behind it (see [`WindowState::frame_interval`]).
    let fps_cap = shared.ui.workspace.interaction.fps_cap;
    let display_max_fps = queue.display_max_fps();
    drop(guard);
    if let Some(interval) = pace(state, fps_cap, display_max_fps) {
        queue.set_frame_interval(interval);
    }
}

/// What the window's frame timer has to be told this frame: the interval the
/// cap and the display ask for, or `None` when it already ticks at that.
///
/// Split out of [`frame`] because it is the whole of the pacing decision and
/// the only part of it a test can reach: a `Queue` — which is where the real
/// frame reads the display and arms the timer — borrows window internals that
/// exist only inside a running window.
fn pace(
    state: &mut WindowState,
    fps_cap: Option<f32>,
    display_max_fps: Option<f64>,
) -> Option<f64> {
    state.arm(target_frame_interval(fps_cap, display_max_fps))
}

/// Seconds between frame-timer ticks when the display won't say how fast it
/// can go. Matches baseview's own default (~67 Hz).
const FALLBACK_FRAME_INTERVAL: f64 = 0.015;

/// How much faster than the display to run the frame timer when uncapped.
///
/// The timer is a plain run-loop timer with no relation to vsync, so a frame
/// is ready when it is ready and the display asks when it asks. Miss the
/// question and you don't lose a little time, you lose a whole refresh — 144
/// becomes 72 for that frame. Averaged over a second that reads as an
/// unsteady 70-100, and a 1.1x margin produced exactly that: 6.31 ms of timer
/// against 6.94 ms of refresh leaves 9% for jitter to eat.
///
/// Why the margin has to be THIS large, which 2x got wrong. A repeating
/// `CFRunLoopTimer` reschedules against its SCHEDULED fire times, not its
/// actual ones: a firing that arrives while the handler is still running is
/// coalesced away, and the next lands on the next grid point in the future.
/// So tick start times are quantized to this interval — and the handler
/// routinely overruns it, because the vsync wait happens INSIDE it
/// (`get_current_texture`, between the uploads and egui's encode). At 2x on a
/// 144 Hz display that grid is 3.47 ms, half a refresh: a tick returning just
/// past a grid point idles for nearly half a refresh before it can start, and
/// the frame it was aiming at is gone. Nothing pulls it back into phase — the
/// grid is fixed and the panel's clock drifts against it — so the phase sweeps
/// and there is a band where the miss is reliable. Measured: an uncapped
/// 144 Hz display sitting at 110-120 fps, which is not a rate at all but a
/// mixture of 6.94 ms and 13.9 ms intervals.
///
/// MEASURED, and it is not the cause. 6x was tried, which puts the
/// quantization at an eighth of a refresh instead of a half, and the frame
/// rate did not move: still 110 fps, with the same drops under load. So the
/// quantization above is real but is not what costs the refreshes — the
/// overlay puts the time in `wait` (4 ms average, 16 ms peak) against an idle
/// GPU and a frame whose own work fits the refresh with room to spare. The
/// stall is in getting a drawable back, not in when the timer fires. Left at
/// 2x accordingly; raising it buys nothing and is not free.
///
/// Not free, because the old "generous is free" note only held while
/// PRESENTING: a tick that renders blocks in acquire, so ticks per second
/// equals refreshes per second whatever this constant says. While IDLE it is
/// false. `on_frame` gates only `render()` on a repaint being due; the full
/// `run_ui` pass — the dock and every pane — runs on every tick regardless
/// (vendored egui-baseview, `window.rs`). An idle editor repaints at 20 Hz
/// but ticks at this rate, so raising the constant multiplies the UI build of
/// a plugin that is only sitting in a project: 288 passes a second at 2x
/// against 864 at 6x, nearly all for frames that will never be drawn.
///
/// In a HOST that idle path is nearly unreachable, so do not weigh it too
/// heavily. `animating` includes `spectrum.is_flowing`, which only means
/// samples arrived recently — and a DAW streams buffers continuously whether
/// or not anything is playing, silence included. So with the Analyzer showing
/// the spectrogram or the curve, which is the default, the editor stays
/// legitimately live and presents every tick: `run_ui` runs at the refresh
/// rate, not at this constant. Measured in Bitwig with nothing playing:
/// "144 fps live", a 7 ms frame. The idle arithmetic above only applies with
/// the Analyzer closed, where the cost is small anyway.
///
/// What the margin is also protecting against: the tick is on the HOST'S main
/// run loop, not a thread of ours. The host's own UI work delays it by however
/// long it likes, and no amount of making our frame cheaper touches that.
///
/// HISTORICAL NOTE, because the original case for 2x no longer holds up. It
/// blamed the 1.1x failure on "jitter and work spikes", and the spikes were
/// real — but they were a bug, not a workload: every frame was reconfiguring
/// the swapchain (see PATCHES.md, patch 9), which cost 0.5-3 ms wandering
/// against 0.63 ms of margin. That is fixed — but the conclusion drawn from
/// it, that a TIGHTER value might hold today, had the sign backwards: the
/// quantization above is what bites, and it wants a looser one.
///
/// To re-test it, watch the overlay's `frame` PEAK, not the fps number. A
/// missed refresh is not a slow frame, it is a DOUBLED interval, so peak at
/// roughly twice avg means misses and peak near avg means the margin is
/// enough. The fps number averages the misses away, which is precisely why
/// the failure first showed up as a vague "unsteady 70-100" rather than as
/// something you could point at.
///
/// The real fix is to stop guessing and drive frames from the display itself
/// (`CVDisplayLink`), which is a much larger change to the vendored baseview —
/// and which would retire the idle cost above along with the quantization.
const DISPLAY_OVERSAMPLE: f64 = 2.0;

/// The interval the window's frame timer should run at.
///
/// Two bounds, whichever is slower: the user's cap, and what the display can
/// actually present. A cap above the refresh rate buys nothing but wasted
/// frames, and a display faster than the cap is exactly what the cap is for.
///
/// Which one binds is decided on RATES, not on the intervals derived from
/// them, because the two intervals are not the same kind of quantity: the
/// display's is deliberately [`DISPLAY_OVERSAMPLE`] times faster than the
/// refresh, and the cap's is the bare rate the user asked for. Comparing them
/// with `max` quietly conflates the two, and the bug only stays hidden while
/// the oversample happens to be small enough that the display's interval is
/// still the longer number. At 6x it is not: a 144 fps cap on a 60 Hz panel
/// should pace off the panel, and under `max` it would instead win with its
/// bare 6.94 ms and throw the whole margin away. That is a cap the user set
/// to mean "don't limit me" making the pacing worse.
///
/// KNOWN GAP: a BINDING cap still throws [`DISPLAY_OVERSAMPLE`] away. A 30 fps
/// cap on a 60 Hz display runs a bare 33.33 ms timer against a 16.67 ms
/// refresh — zero margin, which is the same condition that made a 1.1x
/// oversample unsteady. Any jitter and the frame slips to the third vsync: 20
/// fps for that frame, averaging out as an unsteady 20-30.
///
/// Raising the constant cannot fix that, because capping is a different
/// mechanism from pacing. Nothing throttles a slow timer TO the cap — vsync
/// throttles to 60, not to 30 — so hitting 30 means an oversampled timer plus
/// deliberately skipping presents, not a timer slow enough to land on the
/// right refresh by luck.
pub(super) fn target_frame_interval(fps_cap: Option<f32>, display_max_fps: Option<f64>) -> f64 {
    let display_hz = display_max_fps.filter(|hz| hz.is_finite() && *hz > 0.0);
    let cap = fps_cap.filter(|fps| fps.is_finite() && *fps > 0.0).map(|fps| fps as f64);
    match (cap, display_hz) {
        // The cap binds only when it asks for a SLOWER rate than the panel can
        // show. Then it is itself the pacing mechanism, so it gets no
        // oversample — see the known gap above.
        (Some(fps), Some(hz)) if fps < hz => 1.0 / fps,
        // A cap at or above the refresh rate does not bind at all; pace off
        // the display exactly as if nothing were capped.
        (_, Some(hz)) => 1.0 / (hz * DISPLAY_OVERSAMPLE),
        // No usable refresh rate to compare against, so the cap is all there
        // is — floored at the fallback, which is what an unknown display is
        // assumed to manage.
        (Some(fps), None) => (1.0 / fps).max(FALLBACK_FRAME_INTERVAL),
        (None, None) => FALLBACK_FRAME_INTERVAL,
    }
}

/// State handed to egui-baseview's run loop.
pub(super) struct WindowState {
    pub(super) shared: Arc<Mutex<EditorShared>>,
    pub(super) params: Arc<HarmonigraphParams>,
    /// The frame interval armed on THIS window's timer, so an unchanged
    /// cadence doesn't rebuild the run-loop timer every frame. `None` until
    /// the first frame arms one.
    ///
    /// Per window rather than alongside the rest of the editor's state, and
    /// that placement is the whole of it: a window opens at baseview's
    /// `DEFAULT_FRAME_INTERVAL` (~67 Hz) whatever the window before it was
    /// running at, so a record that outlived the closed window would agree
    /// with the target the first frame computes, skip the arming, and leave a
    /// reopened editor ticking at the default until something else moved the
    /// target. That is a stuck 67 fps that only a cap change clears, and
    /// clears by accident.
    frame_interval: Option<f64>,
}

impl WindowState {
    pub(super) fn new(shared: Arc<Mutex<EditorShared>>, params: Arc<HarmonigraphParams>) -> Self {
        WindowState { shared, params, frame_interval: None }
    }

    /// The interval to arm on the window's frame timer, or `None` when it
    /// already ticks at `target` and re-arming would be pure churn.
    fn arm(&mut self, target: f64) -> Option<f64> {
        if self.frame_interval == Some(target) {
            return None;
        }
        self.frame_interval = Some(target);
        Some(target)
    }
}

#[cfg(test)]
mod tests {
    use super::super::shared::EditorShared;
    use super::{
        pace, target_frame_interval, WindowState, DISPLAY_OVERSAMPLE, FALLBACK_FRAME_INTERVAL,
    };
    use crate::HarmonigraphParams;
    use std::sync::Arc;

    #[test]
    fn a_cap_sets_the_interval_it_names() {
        // 30 fps on a 144 Hz display: the cap binds, exactly, with no
        // rounding to whatever the timer happens to tick at.
        let interval = target_frame_interval(Some(30.0), Some(144.0));
        assert!((interval - 1.0 / 30.0).abs() < 1e-12, "got {interval}");
    }

    #[test]
    fn uncapped_runs_ahead_of_the_display() {
        let interval = target_frame_interval(None, Some(144.0));
        assert!((interval - 1.0 / (144.0 * DISPLAY_OVERSAMPLE)).abs() < 1e-12, "got {interval}");
        assert!(
            interval < 1.0 / 144.0,
            "must oversample, not match, the refresh rate — matching it leaves no margin \
             for jitter, and a missed refresh costs a whole frame",
        );
    }

    /// A window opens at baseview's `DEFAULT_FRAME_INTERVAL` whatever the
    /// window before it was running at, so what is armed cannot be remembered
    /// anywhere that outlives one window: the reopened editor's first frame
    /// asks for the same interval the closed one ended on, and a carried-over
    /// record would match it, skip the arming, and leave the timer at the
    /// ~67 Hz default until a cap change happened to move the target.
    ///
    /// The guarantee being measured is that [`WindowState`] — built once per
    /// [`Editor::spawn`] — is where that record lives, so two of them stand
    /// for two openings of the editor over one plugin.
    #[test]
    fn a_reopened_window_arms_its_own_timer() {
        // Uncapped on a 144 Hz panel, which is where the stuck rate showed.
        let (cap, display) = (None, Some(144.0));
        let target = target_frame_interval(cap, display);

        let mut first = a_window();
        assert_eq!(pace(&mut first, cap, display), Some(target), "the first frame arms nothing");
        assert_eq!(pace(&mut first, cap, display), None, "an unchanged cadence rebuilt the timer");

        // That window closes and the editor opens again over the same plugin.
        let mut second = a_window();
        assert_eq!(
            pace(&mut second, cap, display),
            Some(target),
            "a reopened window was left ticking at baseview's default",
        );
    }

    /// A `WindowState` standing for one opened window.
    ///
    /// What it is built over is not what any pacing test measures — the
    /// decision reads the cap and the display and touches neither the rings
    /// nor the params — so this exists only to satisfy the constructor, in one
    /// place rather than once per test.
    fn a_window() -> WindowState {
        let (_producer, consumer) = harmonigraph_record::publication::channel();
        let (_audio_producer, audio_consumer) = crate::audio_ingress::channel(1);
        let (_recorder, take_control) = harmonigraph_record::channel();
        WindowState::new(
            Arc::new(super::Mutex::new(EditorShared::new(
                consumer,
                audio_consumer,
                Arc::new(super::AtomicU32::new(48_000.0f32.to_bits())),
                take_control,
            ))),
            Arc::new(HarmonigraphParams::default()),
        )
    }

    #[test]
    fn the_slower_of_cap_and_display_wins() {
        // A cap above what the display can show buys nothing: pacing follows
        // the 60 Hz panel, not the 144 the user asked to be allowed.
        let interval = target_frame_interval(Some(144.0), Some(60.0));
        let from_display = 1.0 / (60.0 * DISPLAY_OVERSAMPLE);
        assert!((interval - from_display).abs() < 1e-12, "got {interval}");
    }

    /// A cap that does not bind must not change the pacing AT ALL — picking
    /// "144" on a 60 Hz panel has to be indistinguishable from "Uncapped".
    ///
    /// Its own test because the old `max` over the two intervals only got this
    /// right by accident: the display's interval is oversampled and the cap's
    /// is bare, so which one is numerically larger depends on
    /// [`DISPLAY_OVERSAMPLE`]. At 2x the display's 8.33 ms still beat a 144 fps
    /// cap's 6.94 ms and the cap looked ignored; at 6x it stopped winning and
    /// a non-binding cap started halving the margin. Spelled against the
    /// uncapped result so it cannot drift with the constant again.
    #[test]
    fn a_cap_above_the_refresh_rate_paces_exactly_as_if_uncapped() {
        for hz in [60.0, 120.0, 144.0] {
            for cap in [hz as f32, hz as f32 + 1.0, 240.0, 1000.0] {
                assert_eq!(
                    target_frame_interval(Some(cap), Some(hz)),
                    target_frame_interval(None, Some(hz)),
                    "a {cap} fps cap changed the pacing of a {hz} Hz display",
                );
            }
        }
        // And one that DOES bind still binds, so the guard above isn't just
        // swallowing every cap.
        assert!(
            target_frame_interval(Some(30.0), Some(60.0)) > target_frame_interval(None, Some(60.0)),
            "a binding cap must still slow the timer down",
        );
    }

    #[test]
    fn an_unknown_display_falls_back_rather_than_racing() {
        assert_eq!(target_frame_interval(None, None), FALLBACK_FRAME_INTERVAL);
        // A cap still binds when the display is unknown.
        let interval = target_frame_interval(Some(30.0), None);
        assert!((interval - 1.0 / 30.0).abs() < 1e-12, "got {interval}");
    }

    /// Nothing may talk the timer into spinning the run loop.
    const MIN_SANE_INTERVAL: f64 = 1.0 / 1000.0;

    #[test]
    fn nonsense_caps_and_rates_do_not_produce_a_runaway_timer() {
        // A hand-edited persist blob or a lying screen must not talk the
        // timer into spinning; every one of these falls back to a sane rate.
        // The invariant is "fall back to what the display asks for", not any
        // particular number — spelled against the uncapped result so that
        // retuning DISPLAY_OVERSAMPLE can't quietly turn this into a tautology.
        for bad_cap in [0.0, -1.0, f32::NAN] {
            let interval = target_frame_interval(Some(bad_cap), Some(60.0));
            assert_eq!(interval, target_frame_interval(None, Some(60.0)), "cap {bad_cap}");
            assert!(interval >= MIN_SANE_INTERVAL, "cap {bad_cap} gave {interval}");
        }
        for bad_hz in [0.0, -60.0, f64::NAN] {
            let interval = target_frame_interval(None, Some(bad_hz));
            assert_eq!(interval, FALLBACK_FRAME_INTERVAL, "hz {bad_hz}");
        }
    }
}
