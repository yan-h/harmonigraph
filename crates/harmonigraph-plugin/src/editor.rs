//! A nice-plug `Editor` implementation running egui on egui-baseview's
//! **wgpu** backend (nice-plug-egui defaults to OpenGL, which is deprecated
//! on macOS and can't host our wgpu paint callbacks; it also doesn't opt in
//! to host->plugin resizing). Adapted from nice-plug-egui's editor glue
//! (ISC licensed).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use baseview::{Size, WindowHandle, WindowScalePolicy};
use crossbeam::atomic::AtomicCell;
use egui::Context;
use egui_baseview::{EguiWindow, EguiWindowSettings, GraphicsConfig, Queue, SizeSource};
use harmonigraph_core::notes::NoteEvent as CoreNoteEvent;
use harmonigraph_ui::SharedState;
use nice_plug::prelude::{Editor, GuiContext, ParamSetter, ParentWindowHandle, ResizeHint};
use parking_lot::Mutex;
use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};
use serde::{Deserialize, Serialize};

use crate::{HarmonigraphParams, PluginParamBackend};

/// The surface format egui-baseview's wgpu backend will pick. It isn't
/// exposed through its API, so we mirror the choice egui-wgpu makes: the
/// first supported non-sRGB 8-bit format, which is Bgra8Unorm on both Metal
/// and DX12. If the lattice pane ever panics with a pipeline/surface format
/// mismatch on some exotic setup, this is the knob (the real fix is
/// upstreaming RenderState access in egui-baseview).
const ASSUMED_SURFACE_FORMAT: harmonigraph_render::wgpu::TextureFormat =
    harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm;

/// Editor size on first open, in logical pixels.
pub(crate) const DEFAULT_SIZE: (u32, u32) = (1000, 700);
/// Smallest size accepted from a host resize.
///
/// The width is not this editor's own number: it is the floor the pane layout
/// dials to as well, so it comes from `harmonigraph_ui::shell` rather than
/// being restated here — see
/// [`MIN_WINDOW_WIDTH`](harmonigraph_ui::shell::MIN_WINDOW_WIDTH) for what a
/// window and a layout holding two different floors would cost. The height is
/// nobody else's business.
const MIN_SIZE: (u32, u32) = (harmonigraph_ui::shell::MIN_WINDOW_WIDTH as u32, 300);

/// Maps audio-clock timestamps (seconds on the plugin's sample clock)
/// onto the GUI clock. A smoothed offset estimate preserves the relative
/// spacing of events - re-stamping on arrival would squash every event in a
/// batch onto the same GUI frame time.
pub(crate) struct ClockMapper {
    /// Estimated `gui_time - audio_time` (includes average delivery
    /// latency, which is fine: it's constant-ish, so spacing survives).
    offset: Option<f64>,
}

impl ClockMapper {
    /// An offset jump larger than this means the audio clock restarted
    /// (transport reset, sample-rate change): snap instead of smoothing.
    const SNAP_THRESHOLD: f64 = 1.0;
    const SMOOTHING: f64 = 0.05;

    pub fn new() -> Self {
        ClockMapper { offset: None }
    }

    /// Feed one observation per drained batch: the newest audio timestamp
    /// in the batch against the current GUI time.
    pub fn observe(&mut self, newest_audio_time: f64, gui_now: f64) {
        let candidate = gui_now - newest_audio_time;
        self.offset = Some(match self.offset {
            None => candidate,
            Some(prev) if (candidate - prev).abs() > Self::SNAP_THRESHOLD => candidate,
            Some(prev) => prev + (candidate - prev) * Self::SMOOTHING,
        });
    }

    /// Map an audio timestamp to GUI time (clamped: never in the future).
    pub fn map(&self, audio_time: f64, gui_now: f64) -> f64 {
        match self.offset {
            Some(offset) => (audio_time + offset).min(gui_now),
            None => gui_now,
        }
    }
}

/// State shared between the plugin (which owns the ring buffer producer)
/// and the GUI thread. Lives for the whole plugin lifetime; the editor
/// window may open and close many times around it.
pub struct EditorShared {
    consumer: rtrb::Consumer<CoreNoteEvent>,
    /// Interleaved input frames from the audio thread (Spectral pane analyzer);
    /// `audio_channels` samples each.
    audio_consumer: rtrb::Consumer<f32>,
    /// Sample rate of those samples, as f32 bits (see the plugin struct).
    sample_rate_bits: Arc<AtomicU32>,
    /// Channels per frame in `audio_consumer`, as the audio thread last saw the
    /// bus. Read every drain, never cached: de-interleaving by a stale count
    /// would read one channel as two.
    audio_channels: Arc<AtomicU32>,
    /// Crate-visible because [`crate::background`] shares this state with the
    /// frame — it logs a failure to start into the same console, and its tests
    /// read back the history it filled.
    pub(crate) ui: SharedState,
    /// GUI clock epoch; audio event times are mapped onto this clock.
    start: Instant,
    /// Audio->GUI clock mapping (see ClockMapper).
    clock: ClockMapper,
    /// Reused per-frame drain scratch (events are batched so the clock
    /// observation can use the newest timestamp before mapping).
    drain_buf: Vec<CoreNoteEvent>,
    /// Reused per-frame audio drain scratch.
    audio_buf: Vec<f32>,
    /// When the previous GUI update ran; used to detect event-loop stalls.
    last_frame: Option<Instant>,
    /// The frame interval currently armed on the window, so an unchanged
    /// cadence doesn't rebuild the run-loop timer every frame. `None` until
    /// the first frame sets one.
    frame_interval: Option<f64>,
    /// Param key currently inside a begin_set/end_set automation gesture.
    gesture: std::cell::Cell<Option<harmonigraph_ui::params::ParamKey>>,
    /// Take recording, driven from the Video pane's toggle.
    take: harmonigraph_record::Control,
    /// Events the audio thread has recorded into the current take.
    take_events: Arc<std::sync::atomic::AtomicU64>,
    /// Whether the transport was rolling as of the last recorded event,
    /// for the status line. Derived, not authoritative.
    take_rolling: bool,
    /// Event count at the previous frame; a rise means the transport is
    /// rolling and the audio thread is actually capturing.
    take_last_count: u64,
    /// Consecutive frames the transport has been stopped for, while a
    /// take is recording. Debounces the OnTransportStop trigger: a host
    /// reporting one still block mid-playback must not end a take.
    take_still_frames: u32,
}

impl EditorShared {
    pub fn new(
        consumer: rtrb::Consumer<CoreNoteEvent>,
        audio_consumer: rtrb::Consumer<f32>,
        sample_rate_bits: Arc<AtomicU32>,
        audio_channels: Arc<AtomicU32>,
        take: harmonigraph_record::Control,
        take_events: Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        EditorShared {
            consumer,
            audio_consumer,
            sample_rate_bits,
            audio_channels,
            ui: SharedState::new(ASSUMED_SURFACE_FORMAT),
            start: Instant::now(),
            clock: ClockMapper::new(),
            drain_buf: Vec::new(),
            audio_buf: Vec::new(),
            last_frame: None,
            frame_interval: None,
            gesture: std::cell::Cell::new(None),
            take,
            take_events,
            take_rolling: false,
            take_last_count: 0,
            take_still_frames: 0,
        }
    }

    /// The host sample rate the audio thread published, as an f32. It lives
    /// in an `AtomicU32` as the f32's bit pattern (a lock-free f32); this
    /// names the bit-cast so its readers can't spell it inconsistently.
    fn sample_rate(&self) -> f32 {
        f32::from_bits(self.sample_rate_bits.load(Ordering::Relaxed))
    }

    /// Channels per frame in the audio ring (at least one).
    fn audio_channels(&self) -> usize {
        (self.audio_channels.load(Ordering::Relaxed) as usize).max(1)
    }

    /// Frames the transport must be still before OnTransportStop ends a
    /// take. At the editor's repaint rate this is a fraction of a second
    /// — long enough to ride out a host reporting one stalled block,
    /// short enough that the render feels immediate.
    const STOP_FRAMES: u32 = 20;

    /// Reflect the Video pane's toggle into the recorder, and the
    /// recorder's progress back into the pane. Called once per frame,
    /// before `root_ui` reads the state it draws.
    fn sync_take(&mut self, sample_rate: f32) {
        // The plugin can record; the control is hidden in shells that
        // can't (the standalone uses an env var instead).
        self.ui.take.supported = true;
        let recording = self.take.is_recording();
        if self.ui.take.recording && !recording {
            // Start from the CURRENT look, not the last-saved one: what
            // is on screen right now is what the render should reproduce.
            self.take_events.store(0, std::sync::atomic::Ordering::Relaxed);
            self.take_last_count = 0;
            // `audio: true` unconditionally, rather than from a setting: the
            // render uses the plugin's input as the spectrogram, aligned to the
            // picture by construction (no bounce, no offset). Silent-but-harmless
            // if the plugin sits where no audio reaches it.
            self.take.start(sample_rate, self.ui.save_persist(), true);
        } else if !self.ui.take.recording && recording {
            self.take.stop(harmonigraph_record::RenderRequest::from_config(
                &self.ui.take.render_config,
            ));
        }

        // "Re-render take": render the last finished take with the CURRENT settings.
        // The persist blob rides along as --ui-state, so the frame, bounce, and
        // offset dialed in after recording all reach the video.
        self.ui.take.last_ready = self.take.last_take().is_some();
        if std::mem::take(&mut self.ui.take.render_now) {
            self.take.render_now(harmonigraph_record::RenderRequest::render_now(
                &self.ui.take.render_config,
                self.ui.save_persist(),
            ));
        }

        let count = self.take_events.load(std::sync::atomic::Ordering::Relaxed);
        self.take_last_count = count;
        // The audio thread's own view, rather than inferring it from
        // events arriving: music has gaps, and a gap is not a stop.
        self.take_rolling = self.take.is_rolling();

        // Tell the audio thread whether to end the take at the first loop wrap.
        self.take.set_stop_at_loop_end(
            self.ui.take.render_config.trigger == harmonigraph_ui::RenderTrigger::AtLoopEnd,
        );

        // AtLoopEnd: the audio thread reached the loop end and ended the take
        // (one pass captured, exactly at the loop boundary). Reflect it in the
        // toggle and render that pass.
        if self.take.is_recording()
            && self.ui.take.render_config.trigger == harmonigraph_ui::RenderTrigger::AtLoopEnd
            && self.take.hit_loop_end()
        {
            self.ui.take.recording = false;
            self.take.stop(harmonigraph_record::RenderRequest::from_config(
                &self.ui.take.render_config,
            ));
        }

        // "The take is done" as soon as the transport stops, if asked —
        // so a play-through or an audio export yields a video with
        // nothing further to click.
        if self.take.is_recording()
            && self.ui.take.render_config.trigger == harmonigraph_ui::RenderTrigger::OnTransportStop
        {
            // Only after something was actually captured: arming ahead of
            // the downbeat must not immediately end the take.
            if self.take_rolling || count == 0 {
                self.take_still_frames = 0;
            } else {
                self.take_still_frames += 1;
                if self.take_still_frames >= Self::STOP_FRAMES {
                    self.ui.take.recording = false;
                    self.take.stop(harmonigraph_record::RenderRequest::from_config(
                        &self.ui.take.render_config,
                    ));
                }
            }
        } else {
            self.take_still_frames = 0;
        }
        self.take.tick(self.take_rolling, count);
        self.ui.take.status = self.take.status();
        self.ui.take.render_progress = self.take.render_progress();
        // The shell may have refused to start (unwritable directory);
        // don't leave the indicator claiming otherwise.
        self.ui.take.recording = self.take.is_recording();
        // Steady dot vs. breathing one: whether capture is actually happening.
        self.ui.take.rolling = self.take_rolling;
    }

    /// Record a GUI frame, logging a console warning when the event loop
    /// stalled since the previous one (run-loop mode issues, host
    /// blocking, ...) so freezes stay attributable.
    fn note_frame(&mut self) {
        let gap = self.last_frame.map(|t| t.elapsed().as_secs_f64());
        self.last_frame = Some(Instant::now());
        if let Some(gap) = gap.filter(|g| *g > 0.1) {
            self.ui
                .console
                .log(format!("frame stall: {:.0} ms between updates", gap * 1000.0));
        }
    }

    /// Drain note events from the audio thread into the tracker, mapping
    /// their sample-clock timestamps onto the GUI clock. The batch is
    /// collected FIRST so the mapper can observe the newest timestamp
    /// before mapping any event — that ordering is what preserves
    /// intra-batch spacing (a fast run of notes must not quantize to GUI
    /// frames). Returns true when events arrived, in which case the
    /// caller should repaint this tick rather than at the idle poll.
    fn drain_into_tracker(&mut self, now: f64) -> bool {
        self.drain_buf.clear();
        while let Ok(event) = self.consumer.pop() {
            self.drain_buf.push(event);
        }
        let Some(newest) = self.drain_buf.last() else {
            return false;
        };
        self.clock.observe(newest.time, now);
        for event in &self.drain_buf {
            let mut event = *event;
            event.time = self.clock.map(event.time, now);
            self.ui.tracker.handle_event(event);
        }
        true
    }

    /// Drain the audio sample ring into the spectrum analyzer.
    ///
    /// Always drains AND always feeds. There is no display state that makes the
    /// samples unwanted: the curve and the spectrogram read this one analyzer,
    /// the curve is always drawn, and feeding it is also what drives smooth
    /// repaint (via `is_flowing`). Draining without feeding would only leave the
    /// ring holding stale audio to burst later — and gating the feed on
    /// something being on screen is what [`crate::background`] exists to say is
    /// wrong, since the whole point there is to analyze when nothing shows it.
    fn drain_audio(&mut self, now: f64) {
        self.audio_buf.clear();
        while let Ok(sample) = self.audio_consumer.pop() {
            self.audio_buf.push(sample);
        }
        if !self.audio_buf.is_empty() {
            let sample_rate = self.sample_rate();
            let channels = self.audio_channels();
            let config = self.ui.spectrum_config;
            self.ui.spectrum.push_samples(&self.audio_buf, channels, sample_rate, now, &config);
        }
    }

    /// The clock everything the editor stamps is measured on: seconds since
    /// the PLUGIN was instantiated, not since the window opened.
    ///
    /// That distinction is what makes a closed window recoverable at all. The
    /// clock runs across one, so a column analyzed while nothing was drawing
    /// lands on the same axis as the columns either side of it, and the
    /// heatmap has no seam to hide.
    pub(crate) fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    /// Drain both audio-thread rings onto that clock — notes into the tracker,
    /// samples into the analyzer — and say whether any note arrived.
    ///
    /// A frame calls this, and so does [`crate::background`] while the window
    /// is closed. That they share ONE path is the property the whole
    /// closed-window capture rests on: a second, parallel analyzer would give
    /// back a heatmap on its own hop grid, its own anchor and its own window,
    /// and nothing on screen would say which stretch came from which. Here the
    /// picture is the same picture whether or not anyone was watching it
    /// arrive.
    ///
    /// The notes are drained FIRST so a frame can ask for its repaint before
    /// spending the audio's FFTs; the two are otherwise independent.
    ///
    /// This is HALF of what a drain owes the tracker. Feeding it is here;
    /// ageing it is in [`harmonigraph_ui::begin_frame`], which a frame reaches
    /// through `root_ui` and a caller that never draws does not reach at all.
    /// Anything draining without going on to draw wants
    /// [`catch_up_unwatched`](Self::catch_up_unwatched) instead.
    pub(crate) fn catch_up(&mut self, now: f64) -> bool {
        let notes = self.drain_into_tracker(now);
        self.drain_audio(now);
        notes
    }

    /// The whole of what a drain owes when no frame will follow it: both rings
    /// taken, and then the tracker aged.
    ///
    /// Ageing is not optional bookkeeping. Every note-off parks a voice in
    /// `NoteTracker`'s released tail, and [`prune`] is the only
    /// thing that empties it — so a drainer that fed the tail without pruning
    /// would grow it for as long as the plugin ran, which is exactly the
    /// accumulation `notes.rs` designs its envelope around ("the released tail
    /// would accumulate for the whole session against an O(nodes x voices)
    /// loop"). The roll's own age trim rides on the same call.
    ///
    /// It lives here rather than inside [`catch_up`](Self::catch_up) so a frame
    /// keeps pruning EXACTLY once, in `begin_frame`, against the parameter
    /// mirrors that frame just refreshed. Pruning on the way in as well would
    /// judge the fade against the previous frame's envelope, and a fade time
    /// being dragged UPWARD would drop voices that the new, longer envelope
    /// still has something to draw.
    ///
    /// The envelope here is whatever the last frame left behind — its default
    /// until one has run. That is the right stale value to hold: it is the fade
    /// the picture was last drawn with, and the alternative is not ageing at
    /// all.
    ///
    /// [`prune`]: harmonigraph_core::NoteTracker::prune
    pub(crate) fn catch_up_unwatched(&mut self, now: f64) {
        self.catch_up(now);
        let envelope = self.ui.view.envelope(&self.ui.frame_params);
        self.ui.tracker.prune(now, &envelope);
    }
}

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
fn frame(
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
    let mut shared = state.shared.lock();
    let shared = &mut *shared;

    shared.note_frame();

    let now = shared.now();
    if shared.catch_up(now) {
        // New MIDI must render this tick, not at the idle poll.
        ui.ctx().request_repaint();
    }
    let sample_rate = shared.sample_rate();
    shared.sync_take(sample_rate);

    let backend = PluginParamBackend {
        params: &state.params,
        setter: &setter,
        gesture: &shared.gesture,
    };
    // Last frame's costs that the shell measures and the UI cannot: they
    // happen after `root_ui` returns.
    shared.ui.instruments.timings.tess_ms = queue.tess_ms();
    shared.ui.instruments.timings.egui_gpu_ms = queue.egui_gpu_ms();
    shared.ui.instruments.timings.acquire_ms = queue.acquire_ms();
    shared.ui.instruments.timings.tick_ms = queue.tick_ms();
    shared.ui.instruments.timings.render_ms = queue.render_ms();
    shared.ui.instruments.timings.upload_ms = queue.upload_ms();
    shared.ui.instruments.timings.ubuf_ms = queue.ubuf_ms();
    shared.ui.instruments.timings.texture_ms = queue.texture_ms();
    shared.ui.instruments.timings.prims = queue.prims();
    shared.ui.instruments.timings.verts = queue.verts();
    shared.ui.instruments.timings.encode_ms = queue.encode_ms();
    shared.ui.instruments.timings.submit_ms = queue.submit_ms();
    shared.ui.instruments.timings.shell_ms = shell_start.elapsed().as_secs_f32() * 1000.0;
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
    // effect on the next tick rather than the one after it.
    let target = target_frame_interval(shared.ui.fps_cap, queue.display_max_fps());
    if shared.frame_interval != Some(target) {
        shared.frame_interval = Some(target);
        queue.set_frame_interval(target);
    }
}

/// The wgpu setup, which is `GraphicsConfig::default()` plus a request for
/// timestamp queries so the performance overlay can report GPU time.
///
/// Requested only where the adapter already advertises them, because
/// `request_device` FAILS on an unsupported feature — asking unconditionally
/// would trade a missing readout for a plugin that won't open. Where they
/// aren't granted the overlay simply says "n/a", as it already does for
/// memory on platforms that won't report it.
///
/// TRIED AND REVERTED: `surface.desired_maximum_frame_latency = Some(3)`,
/// which egui-wgpu leaves unset (wgpu defaults it to 2, i.e. one frame of
/// slack). The theory was that a deeper queue would let the CPU work through
/// a delayed present instead of stalling in `get_current_texture`, which is
/// where the overlay puts the time. It made no appreciable difference, and it
/// costs a frame of latency (~7 ms at 144 Hz) on a picture meant to track
/// what you just played — so it is not worth carrying. 3 was the whole knob
/// anyway: wgpu-hal clamps to 2..=3 for `CAMetalLayer.maximumDrawableCount`.
fn graphics_config() -> GraphicsConfig {
    use egui_baseview::WgpuSetup;
    use harmonigraph_render::wgpu;

    let mut config = GraphicsConfig::default();
    if let WgpuSetup::CreateNew(setup) = &mut config.wgpu_options.wgpu_setup {
        let base = setup.device_descriptor.clone();
        setup.device_descriptor = std::sync::Arc::new(move |adapter: &wgpu::Adapter| {
            let mut descriptor = base(adapter);
            descriptor.required_features |=
                adapter.features() & wgpu::Features::TIMESTAMP_QUERY;
            descriptor
        });
    }
    config
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
fn target_frame_interval(fps_cap: Option<f32>, display_max_fps: Option<f64>) -> f64 {
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

pub fn create(
    params: Arc<HarmonigraphParams>,
    shared: Arc<Mutex<EditorShared>>,
) -> Option<Box<dyn Editor>> {
    Some(Box::new(LatticeEditor {
        egui_state: params.editor_state.clone(),
        params,
        shared,
        // On macOS the system reports scaling; elsewhere a host that never
        // calls set_scale_factor gets 1.0 (same policy as nih_plug_egui).
        #[cfg(target_os = "macos")]
        scaling_factor: AtomicCell::new(None),
        #[cfg(not(target_os = "macos"))]
        scaling_factor: AtomicCell::new(Some(1.0)),
    }))
}

/// Window size persistence, modeled on nih_plug_egui's `EguiState`.
#[derive(Debug, Serialize, Deserialize)]
pub struct EguiState {
    /// Size in logical pixels (before DPI scaling).
    #[serde(with = "nice_plug::params::persist::serialize_atomic_cell")]
    size: AtomicCell<(u32, u32)>,
    #[serde(skip)]
    requested_size: AtomicCell<Option<(u32, u32)>>,
    /// A size the host already applied to the parent window (native border
    /// drag); the child view and render surface must be brought to match,
    /// WITHOUT the request_resize round-trip used for plugin-initiated
    /// resizes.
    ///
    /// Collected by the window's size source (see [`LatticeEditor::spawn`])
    /// rather than in the frame, because the frame has to be LAID OUT at this
    /// size, and by the time the frame is running it is too late to say so.
    #[serde(skip)]
    host_resized: AtomicCell<Option<(u32, u32)>>,
    #[serde(skip)]
    open: AtomicBool,
}

impl EguiState {
    pub fn from_size(width: u32, height: u32) -> Arc<EguiState> {
        Arc::new(EguiState {
            size: AtomicCell::new((width, height)),
            requested_size: AtomicCell::new(None),
            host_resized: AtomicCell::new(None),
            open: AtomicBool::new(false),
        })
    }

    pub fn size(&self) -> (u32, u32) {
        self.size.load()
    }

    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    /// Record that the window has opened or closed.
    ///
    /// Named rather than stored inline because it is not only the host's
    /// business: [`crate::background`] reads it to decide whether a frame is
    /// already draining the rings, so the two transitions are a seam between
    /// threads and not just a bit.
    pub(crate) fn set_open(&self, open: bool) {
        self.open.store(open, Ordering::Release);
    }
}

impl<'a> nice_plug::params::persist::PersistentField<'a, EguiState> for Arc<EguiState> {
    fn set(&self, new_value: EguiState) {
        self.size.store(new_value.size.load());
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&EguiState) -> R,
    {
        f(self)
    }
}

struct LatticeEditor {
    egui_state: Arc<EguiState>,
    params: Arc<HarmonigraphParams>,
    shared: Arc<Mutex<EditorShared>>,
    scaling_factor: AtomicCell<Option<f32>>,
}

/// baseview uses a different raw-window-handle version than nih-plug, so
/// the parent handle needs adapting (verbatim from nih_plug_egui).
struct ParentWindowHandleAdapter(ParentWindowHandle);

unsafe impl HasRawWindowHandle for ParentWindowHandleAdapter {
    fn raw_window_handle(&self) -> RawWindowHandle {
        match self.0 {
            ParentWindowHandle::X11Window(window) => {
                let mut handle = raw_window_handle::XcbWindowHandle::empty();
                handle.window = window;
                RawWindowHandle::Xcb(handle)
            }
            ParentWindowHandle::AppKitNsView(ns_view) => {
                let mut handle = raw_window_handle::AppKitWindowHandle::empty();
                handle.ns_view = ns_view;
                RawWindowHandle::AppKit(handle)
            }
            ParentWindowHandle::Win32Hwnd(hwnd) => {
                let mut handle = raw_window_handle::Win32WindowHandle::empty();
                handle.hwnd = hwnd;
                RawWindowHandle::Win32(handle)
            }
        }
    }
}

/// State handed to egui-baseview's run loop.
struct WindowState {
    shared: Arc<Mutex<EditorShared>>,
    params: Arc<HarmonigraphParams>,
}

impl Editor for LatticeEditor {
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn std::any::Any + Send> {
        let egui_state = self.egui_state.clone();
        let (unscaled_width, unscaled_height) = self.egui_state.size();
        let scaling_factor = self.scaling_factor.load();
        // Where the window collects a size the HOST gave it (a border drag),
        // before it builds the frame that has to be laid out at that size. It
        // used to be applied from inside the frame instead, which left every
        // frame of a drag showing content built for the size the window had a
        // moment ago — visible as the whole layout jumping while the border
        // moves, since macOS anchors a child view bottom-left.
        //
        // macOS is also why there is nothing to listen to instead: baseview
        // raises `Resized` there for the initial size and for backing-property
        // changes, and for nothing else — a host resizing the parent view
        // reaches the plugin only as `Editor::set_size`, on another thread.
        let resized = self.egui_state.clone();
        let logging = self.shared.clone();
        let asking = context.clone();
        let size_source = SizeSource::new(move || {
            // OUR ask first, and from here rather than from inside the frame,
            // for the same reason the host's is collected here: the round trip
            // is synchronous, so asking now means the frame about to be built
            // is laid out at the size it asked for instead of the one it is
            // leaving. Asked from inside the frame, the answer arrives after
            // the layout that wanted it and the arrangement wears an extra
            // frame stretched across the old window.
            if let Some(size) = resized.requested_size.load() {
                let started = Instant::now();
                let accepted = asking.request_resize();
                let roundtrip = started.elapsed().as_secs_f64() * 1000.0;
                resized.requested_size.store(None);
                if let Some(mut shared) = logging.try_lock() {
                    shared.ui.console.log(format!(
                        "request_resize {}x{} -> {} ({roundtrip:.1} ms)",
                        size.0,
                        size.1,
                        if accepted { "accepted" } else { "REFUSED" },
                    ));
                }
                if accepted {
                    resized.size.store(size);
                    // The host may have echoed the same size straight back
                    // through `set_size`; taking it here keeps the next frame
                    // from adopting it a second time as though it were a drag.
                    resized.host_resized.swap(None);
                    return Some(Size::new(f64::from(size.0), f64::from(size.1)));
                }
            }
            let (width, height) = resized.host_resized.swap(None)?;
            // The one diagnostic this path had, kept. try_lock rather than
            // lock: the frame's own lock is taken later in the same callback,
            // so this is uncontended in practice, and a resize is not worth
            // blocking a frame for if it ever is not.
            if let Some(mut shared) = logging.try_lock() {
                shared.ui.console.log(format!("host resize {width}x{height}"));
            }
            Some(Size::new(f64::from(width), f64::from(height)))
        });

        let window = EguiWindow::open_parented(
            &ParentWindowHandleAdapter(parent),
            EguiWindowSettings::new()
                .with_logical_size(Size::new(
                    f64::from(unscaled_width),
                    f64::from(unscaled_height),
                ))
                .with_scale_policy(
                    scaling_factor
                        .map(|factor| WindowScalePolicy::ScaleFactor(f64::from(factor)))
                        .unwrap_or(WindowScalePolicy::SystemScaleFactor),
                )
                .with_graphics_config(graphics_config())
                .with_size_source(size_source),
            WindowState {
                shared: self.shared.clone(),
                params: self.params.clone(),
            },
            |egui_ctx: &Context, _queue, state: &mut WindowState| {
                // Everything this context owes the shared UI state, which is
                // NOT new when the context is: the theme, the release of what
                // the closed window left behind, the fold floor, and the
                // layout the host saved with the project (written on the way
                // out, in `LatticeEditorHandle`'s Drop).
                //
                // Cloned out of the lock rather than read across the open, so
                // this holds one lock at a time.
                let serialized = state.params.ui_state.read().clone();
                let mut shared = state.shared.lock();
                harmonigraph_ui::shell::Opening {
                    ctx: egui_ctx,
                    state: &mut shared.ui,
                    persist: Some(&serialized),
                }
                .open();
            },
            // Thin shim: the real per-frame work is `frame`, above. The
            // closure exists only to own `egui_state`/`context` for the
            // window's lifetime.
            move |ui: &mut egui::Ui, queue, state: &mut WindowState| {
                frame(ui, queue, state, &egui_state, context.as_ref());
            },
        );

        self.egui_state.set_open(true);
        Box::new(LatticeEditorHandle {
            egui_state: self.egui_state.clone(),
            shared: self.shared.clone(),
            params: self.params.clone(),
            window,
        })
    }

    fn size(&self) -> (u32, u32) {
        // If a resize was requested but not yet applied, report the
        // requested size so the host resizes to it.
        self.egui_state
            .requested_size
            .load()
            .unwrap_or_else(|| self.egui_state.size())
    }

    // Hosts with proper resize support (Bitwig at least) provide a native
    // window border on this hint. An in-window drag-corner fallback for
    // hosts that ignore it existed until mid-2026 — recover it from git
    // history (`resize_corner` in this file) if such a host turns up.
    fn resize_hint(&self) -> ResizeHint {
        ResizeHint::resizable()
    }

    fn set_size(&self, width: u32, height: u32) -> bool {
        // A set_size with the current size must succeed without side
        // effects (hosts echo plugin-initiated resizes back through here).
        let clamped = (width.max(MIN_SIZE.0), height.max(MIN_SIZE.1));
        if clamped == self.egui_state.size() {
            return true;
        }
        // Report the new size immediately (the host may read size() right
        // after); the GUI thread lays the next frame out at it.
        self.egui_state.size.store(clamped);
        self.egui_state.host_resized.store(Some(clamped));
        true
    }

    fn set_scale_factor(&self, factor: f32) -> bool {
        // Can't rescale while open (Ableton Live does this).
        if self.egui_state.is_open() {
            return false;
        }
        self.scaling_factor.store(Some(factor));
        true
    }

    fn param_value_changed(&self, _id: &str, _normalized_value: f32) {
        // root_ui repaints continuously; nothing to do.
    }

    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {}

    fn param_values_changed(&self) {}
}

struct LatticeEditorHandle {
    egui_state: Arc<EguiState>,
    shared: Arc<Mutex<EditorShared>>,
    params: Arc<HarmonigraphParams>,
    window: WindowHandle,
}

// WindowHandle contains raw pointers, but the handle is only used to close
// the window from the host's GUI thread.
unsafe impl Send for LatticeEditorHandle {}

impl Drop for LatticeEditorHandle {
    fn drop(&mut self) {
        // Persist the UI state (dock layout, camera, view settings) into
        // the plugin state so the host saves it with the project.
        // The lock is taken here with `open` still TRUE, which is what keeps
        // the background analyzer off it for the whole of this — see
        // [`crate::background`] on why that ordering is load-bearing rather
        // than incidental.
        *self.params.ui_state.write() = harmonigraph_ui::shell::close(&self.shared.lock().ui);
        self.egui_state.set_open(false);
        self.window.close();
    }
}


#[cfg(test)]
mod tests {
    use super::{
        target_frame_interval, ClockMapper, EditorShared, DISPLAY_OVERSAMPLE,
        FALLBACK_FRAME_INTERVAL, MIN_SIZE,
    };
    use harmonigraph_core::notes::{NoteEvent, NoteEventKind};

    /// The floor this window is held to and the floor the pane layout dials to
    /// are one number, and the cast into window pixels is where they could
    /// stop being one: a floor with a fraction in it would leave the window
    /// stopping a fraction below what the layout believes, and the layout
    /// banking a difference the window will never give back (see
    /// `harmonigraph_ui::shell::MIN_WINDOW_WIDTH`).
    #[test]
    fn the_window_floor_is_exactly_the_floor_the_layout_dials_to() {
        assert_eq!(MIN_SIZE.0 as f32, harmonigraph_ui::shell::MIN_WINDOW_WIDTH);
    }

    #[test]
    fn drain_observes_the_batch_before_mapping_it() {
        // The audio clock reads ~100s while the GUI clock reads ~7s; the
        // drained voices must land on the GUI clock with their intra-batch
        // spacing intact. (This is the integration the ClockMapper unit
        // tests below can't cover: observe-newest-THEN-map ordering.)
        let (mut producer, consumer) = rtrb::RingBuffer::new(64);
        let (_audio_producer, audio_consumer) = rtrb::RingBuffer::new(64);
        let (_recorder, take_control) = harmonigraph_record::channel();
        let mut shared = EditorShared::new(
            consumer,
            audio_consumer,
            std::sync::Arc::new(super::AtomicU32::new(48_000.0f32.to_bits())),
            std::sync::Arc::new(super::AtomicU32::new(1)),
            take_control,
            std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        );
        for (note, time) in [(60u8, 99.950), (64u8, 99.995)] {
            producer
                .push(NoteEvent {
                    time,
                    channel: 0,
                    note,
                    kind: NoteEventKind::On { velocity: 1.0 },
                })
                .unwrap();
        }

        assert!(shared.drain_into_tracker(7.0), "events arrived -> repaint");
        let mut on_times: Vec<f64> =
            shared.ui.tracker.voices().map(|v| v.on_time).collect();
        on_times.sort_by(f64::total_cmp);
        assert_eq!(on_times.len(), 2);
        assert!(
            (on_times[1] - on_times[0] - 0.045).abs() < 1e-9,
            "spacing lost: {on_times:?}"
        );
        assert!(on_times[1] <= 7.0, "never maps into the GUI future");

        // Empty ring: no work, no repaint request.
        assert!(!shared.drain_into_tracker(7.1));
    }

    /// `catch_up`'s ANSWER, which is the only thing that asks for a repaint on
    /// the tick a note arrives rather than at the idle poll (see `frame`).
    ///
    /// Its own test because the drain and the answer fail independently: every
    /// other test of this path reads the tracker afterwards, so a `catch_up`
    /// that drained perfectly and always answered `false` would satisfy all of
    /// them while costing every note played the latency of the idle poll.
    #[test]
    fn catch_up_answers_whether_notes_arrived() {
        let (mut producer, consumer) = rtrb::RingBuffer::new(64);
        let (_audio_producer, audio_consumer) = rtrb::RingBuffer::new(64);
        let (_recorder, take_control) = harmonigraph_record::channel();
        let mut shared = EditorShared::new(
            consumer,
            audio_consumer,
            std::sync::Arc::new(super::AtomicU32::new(48_000.0f32.to_bits())),
            std::sync::Arc::new(super::AtomicU32::new(1)),
            take_control,
            std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        );

        // An empty ring is not a repaint.
        assert!(!shared.catch_up(7.0), "nothing arrived, so nothing needs drawing");

        producer
            .push(NoteEvent {
                time: 1.0,
                channel: 0,
                note: 60,
                kind: NoteEventKind::On { velocity: 1.0 },
            })
            .unwrap();
        assert!(shared.catch_up(7.1), "a note arrived and the frame was not told");

        // And the ring is empty again, so the next tick asks for nothing.
        assert!(!shared.catch_up(7.2), "the same note asked for a second repaint");
    }

    #[test]
    fn preserves_intra_batch_spacing() {
        let mut clock = ClockMapper::new();
        // Audio clock at ~100s, GUI clock at ~7s.
        clock.observe(100.0, 7.0);
        let a = clock.map(99.950, 7.0);
        let b = clock.map(99.995, 7.0);
        assert!((b - a - 0.045).abs() < 1e-9, "spacing lost: {a} {b}");
    }

    #[test]
    fn never_maps_into_the_future() {
        let mut clock = ClockMapper::new();
        clock.observe(100.0, 7.0);
        assert!(clock.map(120.0, 7.0) <= 7.0);
    }

    #[test]
    fn snaps_on_transport_reset() {
        let mut clock = ClockMapper::new();
        clock.observe(100.0, 7.0);
        // Transport reset: audio clock restarts near zero.
        clock.observe(0.1, 7.5);
        let mapped = clock.map(0.1, 7.5);
        assert!((mapped - 7.5).abs() < 1e-9, "did not snap: {mapped}");
    }

    #[test]
    fn smooths_small_jitter() {
        let mut clock = ClockMapper::new();
        clock.observe(100.0, 7.0); // offset -93
        clock.observe(101.0, 8.1); // candidate -92.9: jitter, not a reset
        let mapped = clock.map(101.0, 9.0);
        // Offset moved only 5% of the way toward the new candidate.
        assert!((mapped - (101.0 - 93.0 + 0.005)).abs() < 1e-9, "got {mapped}");
    }

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
            target_frame_interval(Some(30.0), Some(60.0))
                > target_frame_interval(None, Some(60.0)),
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
