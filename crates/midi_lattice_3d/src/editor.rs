//! A nice-plug `Editor` implementation running egui on egui-baseview's
//! **wgpu** backend (nice-plug-egui defaults to OpenGL, which is deprecated
//! on macOS and can't host our wgpu paint callbacks; it also doesn't opt in
//! to host->plugin resizing). Adapted from nice-plug-egui's editor glue
//! (ISC licensed).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use baseview::{PhySize, Size, WindowHandle, WindowScalePolicy};
use crossbeam::atomic::AtomicCell;
use egui::Context;
use egui_baseview::{EguiWindow, EguiWindowSettings, GraphicsConfig, Queue};
use lattice_core::notes::NoteEvent as CoreNoteEvent;
use lattice_ui::SharedState;
use nice_plug::prelude::{Editor, GuiContext, ParamSetter, ParentWindowHandle, ResizeHint};
use parking_lot::Mutex;
use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};
use serde::{Deserialize, Serialize};

use crate::{MidiLattice3dParams, PluginParamBackend};

/// The surface format egui-baseview's wgpu backend will pick. It isn't
/// exposed through its API, so we mirror the choice egui-wgpu makes: the
/// first supported non-sRGB 8-bit format, which is Bgra8Unorm on both Metal
/// and DX12. If the lattice pane ever panics with a pipeline/surface format
/// mismatch on some exotic setup, this is the knob (the real fix is
/// upstreaming RenderState access in egui-baseview).
const ASSUMED_SURFACE_FORMAT: lattice_render::wgpu::TextureFormat =
    lattice_render::wgpu::TextureFormat::Bgra8Unorm;

/// Editor size on first open, in logical pixels.
pub(crate) const DEFAULT_SIZE: (u32, u32) = (1000, 700);
/// Smallest size accepted from a host resize.
const MIN_SIZE: (u32, u32) = (400, 300);

/// Maps audio-clock timestamps (seconds on the plugin's sample clock)
/// onto the GUI clock. A smoothed offset estimate preserves the relative
/// spacing of events - re-stamping on arrival (the old approach) squashed
/// every event in a batch onto the same GUI frame time.
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
    /// Mono input samples from the audio thread (Spectral pane analyzer).
    audio_consumer: rtrb::Consumer<f32>,
    /// Sample rate of those samples, as f32 bits (see the plugin struct).
    sample_rate_bits: Arc<AtomicU32>,
    ui: SharedState,
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
    gesture: std::cell::Cell<Option<lattice_ui::params::ParamKey>>,
    /// Take recording, driven from the Video pane's toggle.
    take: crate::take::Control,
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
        take: crate::take::Control,
        take_events: Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        EditorShared {
            consumer,
            audio_consumer,
            sample_rate_bits,
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
        self.ui.take_supported = true;
        // Recording always captures the plugin's audio input now: the render
        // uses it as the spectrogram, aligned to the picture by construction (no
        // bounce, no offset). Silent-but-harmless if the plugin sits where no
        // audio reaches it.
        self.ui.take_audio = true;
        let recording = self.take.is_recording();
        if self.ui.take_recording && !recording {
            // Start from the CURRENT look, not the last-saved one: what
            // is on screen right now is what the render should reproduce.
            self.take_events.store(0, std::sync::atomic::Ordering::Relaxed);
            self.take_last_count = 0;
            self.take.start(sample_rate, self.ui.save_persist(), true);
        } else if !self.ui.take_recording && recording {
            self.take.stop(crate::take::RenderRequest::from_config(&self.ui.render_config));
        }

        // "Render now": render the last finished take with the CURRENT settings.
        // The persist blob rides along as --ui-state, so the frame, bounce, and
        // offset dialed in after recording all reach the video.
        self.ui.last_take_ready = self.take.last_take().is_some();
        if std::mem::take(&mut self.ui.render_now) {
            self.take.render_now(crate::take::RenderRequest::render_now(
                &self.ui.render_config,
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
            self.ui.render_config.trigger == lattice_ui::RenderTrigger::AtLoopEnd,
        );

        // AtLoopEnd: the audio thread reached the loop end and ended the take
        // (one pass captured, exactly at the loop boundary). Reflect it in the
        // toggle and render that pass.
        if self.take.is_recording()
            && self.ui.render_config.trigger == lattice_ui::RenderTrigger::AtLoopEnd
            && self.take.hit_loop_end()
        {
            self.ui.take_recording = false;
            self.take.stop(crate::take::RenderRequest::from_config(&self.ui.render_config));
        }

        // "The take is done" as soon as the transport stops, if asked —
        // so a play-through or an audio export yields a video with
        // nothing further to click.
        if self.take.is_recording()
            && self.ui.render_config.trigger == lattice_ui::RenderTrigger::OnTransportStop
        {
            // Only after something was actually captured: arming ahead of
            // the downbeat must not immediately end the take.
            if self.take_rolling || count == 0 {
                self.take_still_frames = 0;
            } else {
                self.take_still_frames += 1;
                if self.take_still_frames >= Self::STOP_FRAMES {
                    self.ui.take_recording = false;
                    self.take.stop(crate::take::RenderRequest::from_config(
                        &self.ui.render_config,
                    ));
                }
            }
        } else {
            self.take_still_frames = 0;
        }
        self.take.tick(self.take_rolling, count);
        self.ui.take_status = self.take.status();
        // The shell may have refused to start (unwritable directory);
        // don't leave the indicator claiming otherwise.
        self.ui.take_recording = self.take.is_recording();
        // Steady dot vs. breathing one: whether capture is actually happening.
        self.ui.take_rolling = self.take_rolling;
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

    /// Drain the audio sample ring into the spectrum analyzer. Always
    /// drains — the ring must not hold stale audio for a burst when the
    /// display is toggled on — but skips the analyzer while nothing shows it.
    fn drain_audio(&mut self, now: f64) {
        self.audio_buf.clear();
        while let Ok(sample) = self.audio_consumer.pop() {
            self.audio_buf.push(sample);
        }
        // Feed it when EITHER the curve or the spectrogram is shown — both read
        // from this one analyzer, so a spectrogram with the curve off still
        // needs samples (and, via `is_flowing`, still drives smooth repaint).
        let shown =
            self.ui.spectrum_config.show_audio || self.ui.spectrum_config.show_spectrogram;
        if shown && !self.audio_buf.is_empty() {
            let sample_rate = self.sample_rate();
            self.ui.spectrum.push_samples(&self.audio_buf, sample_rate, now);
        }
    }
}

/// Logical → physical pixels for queue.resize.
fn physical(size: (u32, u32), scale: f32) -> PhySize {
    PhySize::new(
        (size.0 as f32 * scale).round() as u32,
        (size.1 as f32 * scale).round() as u32,
    )
}

/// Apply window resizes negotiated outside the frame loop. Two paths:
///
/// - Host-initiated (native window border): the parent is already the new
///   size; just bring the child view and render surface along, with no
///   request_resize round-trip.
/// - Plugin-initiated (Editor::set_size): PEEK — don't consume — the
///   requested size, because the host reads `Editor::size()` *during*
///   `request_resize()` and must see the NEW size there. Consuming first
///   (as nih_plug_egui does) makes the host resize the parent to the
///   previous size while the child resizes to the new one; the mismatch
///   shows up as content shifted toward the bottom (macOS anchors child
///   views bottom-left).
///
/// Context::pixels_per_point() is the scale the renderer itself uses; the
/// input's viewport info is not reliably populated on this stack (reading
/// it as 1.0 halves the surface on Retina). queue.resize takes physical
/// pixels and — in our patched egui-baseview — also resizes the child
/// view to the matching logical size (macOS baseview emits no Resized
/// event for programmatic resizes, so that patch is the only thing
/// keeping view and surface in sync).
fn apply_pending_resizes(
    egui_state: &EguiState,
    egui_ctx: &Context,
    queue: &mut Queue,
    context: &dyn GuiContext,
    console: &mut lattice_ui::Console,
) {
    if let Some((w, h)) = egui_state.host_resized.swap(None) {
        let scale = egui_ctx.pixels_per_point();
        queue.resize(physical((w, h), scale));
        console.log(format!("host resize {}x{} (scale {:.2})", w, h, scale));
    }

    if let Some(new_size) = egui_state.requested_size.load() {
        let t0 = Instant::now();
        let accepted = context.request_resize();
        let roundtrip_ms = t0.elapsed().as_secs_f64() * 1000.0;
        console.log(format!(
            "request_resize {}x{} -> {} ({:.1} ms)",
            new_size.0,
            new_size.1,
            if accepted { "accepted" } else { "REFUSED" },
            roundtrip_ms,
        ));
        if accepted {
            queue.resize(physical(new_size, egui_ctx.pixels_per_point()));
            egui_state.size.store(new_size);
        }
        egui_state.requested_size.store(None);
    }
}

/// The plugin's per-frame GUI work, in order: take the frame's lock, close
/// out the note frame, apply any pending host/self resize, drain the MIDI
/// and audio rings, then hand the shared UI state to `lattice_ui::root_ui`.
///
/// The standalone harness's counterpart is `App::ui` in lattice-standalone.
/// Both must feed the tracker and the spectrum BEFORE calling `root_ui`,
/// and pass the same clock that stamped the events — see `root_ui`'s doc.
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
    apply_pending_resizes(egui_state, ui.ctx(), queue, context, &mut shared.ui.console);

    let now = shared.start.elapsed().as_secs_f64();
    if shared.drain_into_tracker(now) {
        // New MIDI must render this tick, not at the idle poll.
        ui.ctx().request_repaint();
    }
    shared.drain_audio(now);
    let sample_rate = shared.sample_rate();
    shared.sync_take(sample_rate);

    let backend = PluginParamBackend {
        params: &state.params,
        setter: &setter,
        gesture: &shared.gesture,
    };
    // Last frame's costs that the shell measures and the UI cannot: they
    // happen after `root_ui` returns.
    shared.ui.tess_ms = queue.tess_ms();
    shared.ui.egui_gpu_ms = queue.egui_gpu_ms();
    shared.ui.acquire_ms = queue.acquire_ms();
    shared.ui.tick_ms = queue.tick_ms();
    shared.ui.render_ms = queue.render_ms();
    shared.ui.shell_ms = shell_start.elapsed().as_secs_f32() * 1000.0;
    lattice_ui::root_ui(ui, &mut shared.ui, &backend, now);

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
fn graphics_config() -> GraphicsConfig {
    use egui_baseview::WgpuSetup;
    use lattice_render::wgpu;

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
/// unsteady 70-100, which is exactly what a 1.1x margin produced: 6.31 ms of
/// timer against 6.94 ms of refresh leaves 9% for jitter and work spikes to
/// eat, and they do.
///
/// 2x is deliberately generous. The extra ticks are not wasted frames: the
/// surface presents with vsync, so a tick that arrives with the swapchain
/// full simply blocks until a slot frees, and the display rate throttles the
/// timer rather than the other way round. Oversampling buys the margin;
/// vsync spends it.
///
/// The real fix is to stop guessing and drive frames from the display itself
/// (`CVDisplayLink`), which is a much larger change to the vendored baseview.
const DISPLAY_OVERSAMPLE: f64 = 2.0;

/// The interval the window's frame timer should run at.
///
/// Two bounds, whichever is slower: the user's cap, and what the display can
/// actually present. A cap above the refresh rate buys nothing but wasted
/// frames, and a display faster than the cap is exactly what the cap is for.
fn target_frame_interval(fps_cap: Option<f32>, display_max_fps: Option<f64>) -> f64 {
    let from_display = match display_max_fps {
        Some(hz) if hz.is_finite() && hz > 0.0 => 1.0 / (hz * DISPLAY_OVERSAMPLE),
        _ => FALLBACK_FRAME_INTERVAL,
    };
    match fps_cap {
        // Longer interval = slower rate, so `max` picks the binding bound.
        Some(fps) if fps.is_finite() && fps > 0.0 => (1.0 / fps as f64).max(from_display),
        _ => from_display,
    }
}

pub fn create(
    params: Arc<MidiLattice3dParams>,
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
    /// drag); the GUI thread must resize the child view/surface to match,
    /// WITHOUT the request_resize round-trip used for plugin-initiated
    /// resizes.
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
    params: Arc<MidiLattice3dParams>,
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
    params: Arc<MidiLattice3dParams>,
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
                .with_graphics_config(graphics_config()),
            WindowState {
                shared: self.shared.clone(),
                params: self.params.clone(),
            },
            |egui_ctx: &Context, _queue, state: &mut WindowState| {
                lattice_ui::theme::apply_theme(egui_ctx);
                // This is a NEW context; the shared UI state is not. Anything
                // it holds that belongs to the last one has to go now, or it
                // silently keeps drawing with handles that point nowhere —
                // which is how the spectrogram disappeared for good after the
                // window was hidden and shown again.
                let mut shared = state.shared.lock();
                shared.ui.release_context_resources();
                // Restore dock layout / camera / view settings persisted
                // with the plugin state (saved when the editor closes).
                let serialized = state.params.ui_state.read().clone();
                if !serialized.is_empty() {
                    shared.ui.load_persist(&serialized);
                }
            },
            // Thin shim: the real per-frame work is `frame`, above. The
            // closure exists only to own `egui_state`/`context` for the
            // window's lifetime.
            move |ui: &mut egui::Ui, queue, state: &mut WindowState| {
                frame(ui, queue, state, &egui_state, context.as_ref());
            },
        );

        self.egui_state.open.store(true, Ordering::Release);
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
        // after); the GUI thread applies it to the view/surface next frame.
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
    params: Arc<MidiLattice3dParams>,
    window: WindowHandle,
}

// WindowHandle contains raw pointers, but the handle is only used to close
// the window from the host's GUI thread.
unsafe impl Send for LatticeEditorHandle {}

impl Drop for LatticeEditorHandle {
    fn drop(&mut self) {
        // Persist the UI state (dock layout, camera, view settings) into
        // the plugin state so the host saves it with the project.
        *self.params.ui_state.write() = self.shared.lock().ui.save_persist();
        self.egui_state.open.store(false, Ordering::Release);
        self.window.close();
    }
}


#[cfg(test)]
mod tests {
    use super::{
        target_frame_interval, ClockMapper, EditorShared, DISPLAY_OVERSAMPLE,
        FALLBACK_FRAME_INTERVAL,
    };
    use lattice_core::notes::{NoteEvent, NoteEventKind};

    #[test]
    fn drain_observes_the_batch_before_mapping_it() {
        // The audio clock reads ~100s while the GUI clock reads ~7s; the
        // drained voices must land on the GUI clock with their intra-batch
        // spacing intact. (This is the integration the ClockMapper unit
        // tests below can't cover: observe-newest-THEN-map ordering.)
        let (mut producer, consumer) = rtrb::RingBuffer::new(64);
        let (_audio_producer, audio_consumer) = rtrb::RingBuffer::new(64);
        let (_recorder, take_control) = crate::take::channel();
        let mut shared = EditorShared::new(
            consumer,
            audio_consumer,
            std::sync::Arc::new(super::AtomicU32::new(48_000.0f32.to_bits())),
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
        // rounding to whatever the timer used to tick at.
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
