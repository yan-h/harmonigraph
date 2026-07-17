//! A nice-plug `Editor` implementation running egui on egui-baseview's
//! **wgpu** backend (nice-plug-egui defaults to OpenGL, which is deprecated
//! on macOS and can't host our wgpu paint callbacks; it also doesn't opt in
//! to host->plugin resizing). Adapted from nice-plug-egui's editor glue
//! (ISC licensed).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use baseview::{PhySize, Size, WindowHandle, WindowScalePolicy};
use crossbeam::atomic::AtomicCell;
use egui::Context;
use egui_baseview::{EguiWindow, EguiWindowSettings, GraphicsConfig};
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
    ui: SharedState,
    /// GUI clock epoch; audio event times are mapped onto this clock.
    start: Instant,
    /// Audio->GUI clock mapping (see ClockMapper).
    clock: ClockMapper,
    /// Reused per-frame drain scratch (events are batched so the clock
    /// observation can use the newest timestamp before mapping).
    drain_buf: Vec<CoreNoteEvent>,
    /// When the previous GUI update ran; used to detect event-loop stalls.
    last_frame: Option<Instant>,
    /// Param key currently inside a begin_set/end_set automation gesture.
    gesture: std::cell::Cell<Option<lattice_ui::params::ParamKey>>,
}

impl EditorShared {
    pub fn new(consumer: rtrb::Consumer<CoreNoteEvent>) -> Self {
        EditorShared {
            consumer,
            ui: SharedState::new(ASSUMED_SURFACE_FORMAT),
            start: Instant::now(),
            clock: ClockMapper::new(),
            drain_buf: Vec::new(),
            last_frame: None,
            gesture: std::cell::Cell::new(None),
        }
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
                .with_graphics_config(GraphicsConfig::default()),
            WindowState {
                shared: self.shared.clone(),
                params: self.params.clone(),
            },
            |egui_ctx: &Context, _queue, state: &mut WindowState| {
                lattice_ui::theme::apply_theme(egui_ctx);
                // Restore dock layout / camera / view settings persisted
                // with the plugin state (saved when the editor closes).
                let serialized = state.params.ui_state.read().clone();
                if !serialized.is_empty() {
                    state.shared.lock().ui.load_persist(&serialized);
                }
            },
            move |ui: &mut egui::Ui, queue, state: &mut WindowState| {
                let egui_ctx = ui.ctx().clone();
                let egui_ctx = &egui_ctx;
                let setter = ParamSetter::new(context.as_ref());

                // Host-negotiated resizing, as in nih_plug_egui: the GUI
                // stores a requested size, we ask the host, and only apply
                // it if the host agrees.
                {
                    // Diagnostics: the GUI is driven by a frame timer, so a
                    // long gap between updates means the event loop stalled
                    // (run-loop mode issues, host blocking, ...). Surface
                    // stalls in the console to make freezes attributable.
                    let mut shared = state.shared.lock();
                    let gap = shared.last_frame.map(|t| t.elapsed().as_secs_f64());
                    shared.last_frame = Some(Instant::now());
                    if let Some(gap) = gap.filter(|g| *g > 0.1) {
                        shared
                            .ui
                            .console
                            .log(format!("frame stall: {:.0} ms between updates", gap * 1000.0));
                    }
                }

                // Host-initiated resize (native window border): the parent
                // is already the new size; just bring the child view and
                // render surface along. No request_resize round-trip.
                if let Some((w, h)) = egui_state.host_resized.swap(None) {
                    // Context::pixels_per_point() is the value the renderer
                    // itself uses; the input's viewport info is not reliably
                    // populated on this stack (reading it as 1.0 halves the
                    // surface on Retina: 2x-zoomed, bottom-left-anchored
                    // content).
                    let scale = egui_ctx.pixels_per_point();
                    // queue.resize takes physical pixels and (in our patched
                    // egui-baseview) also resizes the child view to the
                    // matching logical size, so no separate InnerSize
                    // command is needed.
                    queue.resize(PhySize::new(
                        (w as f32 * scale).round() as u32,
                        (h as f32 * scale).round() as u32,
                    ));
                    state.shared.lock().ui.console.log(format!(
                        "host resize {}x{} (scale {:.2})",
                        w, h, scale
                    ));
                }

                // Peek — don't consume — the requested size: the host reads
                // `Editor::size()` *during* `request_resize()`, and it must
                // see the NEW size there. Consuming first (as nih_plug_egui
                // does) makes the host resize the parent window to the
                // previous size while we resize the child to the new one;
                // the mismatch shows up as content shifted toward the
                // bottom (macOS anchors child views bottom-left).
                if let Some(new_size) = egui_state.requested_size.load() {
                    let t0 = Instant::now();
                    let accepted = context.request_resize();
                    let roundtrip_ms = t0.elapsed().as_secs_f64() * 1000.0;
                    state.shared.lock().ui.console.log(format!(
                        "request_resize {}x{} -> {} ({:.1} ms)",
                        new_size.0,
                        new_size.1,
                        if accepted { "accepted" } else { "REFUSED" },
                        roundtrip_ms,
                    ));
                    if accepted {
                        // queue.resize takes physical pixels; the patched
                        // egui-baseview resizes the render surface AND the
                        // child view (macOS baseview emits no Resized event
                        // for programmatic resizes, so this is the only
                        // thing keeping them in sync).
                        let scale = egui_ctx.pixels_per_point();
                        queue.resize(PhySize::new(
                            (new_size.0 as f32 * scale).round() as u32,
                            (new_size.1 as f32 * scale).round() as u32,
                        ));
                        egui_state.size.store(new_size);
                    }
                    egui_state.requested_size.store(None);
                }

                let mut shared = state.shared.lock();
                let shared = &mut *shared;
                let now = shared.start.elapsed().as_secs_f64();

                // Drain note events from the audio thread and map their
                // sample-clock timestamps onto the GUI clock through the
                // smoothed offset estimate, preserving intra-batch spacing
                // (a fast run of notes no longer quantizes to GUI frames).
                shared.drain_buf.clear();
                while let Ok(event) = shared.consumer.pop() {
                    shared.drain_buf.push(event);
                }
                if let Some(newest) = shared.drain_buf.last() {
                    shared.clock.observe(newest.time, now);
                    let EditorShared { drain_buf, clock, ui, .. } = shared;
                    for event in drain_buf.iter() {
                        let mut event = *event;
                        event.time = clock.map(event.time, now);
                        ui.tracker.handle_event(event);
                    }
                    // New MIDI must render this tick, not at the idle poll.
                    egui_ctx.request_repaint();
                }

                let backend = PluginParamBackend {
                    params: &state.params,
                    setter: &setter,
                    gesture: &shared.gesture,
                };
                lattice_ui::root_ui(ui, &mut shared.ui, &backend, now);
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
        let clamped = (width.max(400), height.max(300));
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
    use super::ClockMapper;

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
}
