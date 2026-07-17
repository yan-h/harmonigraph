//! A nih-plug `Editor` implementation running egui on egui-baseview's
//! **wgpu** backend (nih_plug_egui is OpenGL-only, which is deprecated on
//! macOS and can't host our wgpu paint callbacks). Adapted from
//! nih_plug_egui's editor glue (ISC licensed).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use baseview::{PhySize, Size, WindowHandle, WindowOpenOptions, WindowScalePolicy};
use crossbeam::atomic::AtomicCell;
use egui::{Context, ViewportCommand};
use egui_baseview::{EguiWindow, GraphicsConfig};
use lattice_core::notes::NoteEvent as CoreNoteEvent;
use lattice_ui::SharedState;
use nih_plug::prelude::{Editor, GuiContext, ParamSetter, ParentWindowHandle};
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

/// State shared between the plugin (which owns the ring buffer producer)
/// and the GUI thread. Lives for the whole plugin lifetime; the editor
/// window may open and close many times around it.
pub struct EditorShared {
    consumer: rtrb::Consumer<CoreNoteEvent>,
    ui: SharedState,
    /// GUI clock epoch. Note events are re-stamped with this clock when
    /// drained (see below).
    start: Instant,
    /// When the previous GUI update ran; used to detect event-loop stalls.
    last_frame: Option<Instant>,
}

impl EditorShared {
    pub fn new(consumer: rtrb::Consumer<CoreNoteEvent>) -> Self {
        EditorShared {
            consumer,
            ui: SharedState::new(ASSUMED_SURFACE_FORMAT),
            start: Instant::now(),
            last_frame: None,
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
    #[serde(with = "nih_plug::params::persist::serialize_atomic_cell")]
    size: AtomicCell<(u32, u32)>,
    #[serde(skip)]
    requested_size: AtomicCell<Option<(u32, u32)>>,
    #[serde(skip)]
    open: AtomicBool,
}

impl EguiState {
    pub fn from_size(width: u32, height: u32) -> Arc<EguiState> {
        Arc::new(EguiState {
            size: AtomicCell::new((width, height)),
            requested_size: AtomicCell::new(None),
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

impl<'a> nih_plug::params::persist::PersistentField<'a, EguiState> for Arc<EguiState> {
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
            WindowOpenOptions {
                title: String::from("MIDI Lattice 3D"),
                size: Size::new(f64::from(unscaled_width), f64::from(unscaled_height)),
                scale: scaling_factor
                    .map(|factor| WindowScalePolicy::ScaleFactor(f64::from(factor)))
                    .unwrap_or(WindowScalePolicy::SystemScaleFactor),
            },
            GraphicsConfig::default(),
            WindowState {
                shared: self.shared.clone(),
                params: self.params.clone(),
            },
            |_egui_ctx: &Context, _queue, _state: &mut WindowState| {},
            move |egui_ctx: &Context, queue, state: &mut WindowState| {
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
                        // `queue.resize` takes PHYSICAL pixels, and on macOS
                        // baseview never emits a Resized event for
                        // programmatic resizes, so this is the only thing
                        // keeping the render surface in sync. Passing
                        // logical points here (as nih_plug_egui does) shrinks
                        // the surface by the DPI factor on Retina displays:
                        // blurry output and dead mouse regions.
                        let scale = egui_ctx
                            .input(|i| i.viewport().native_pixels_per_point)
                            .unwrap_or(1.0);
                        queue.resize(PhySize::new(
                            (new_size.0 as f32 * scale).round() as u32,
                            (new_size.1 as f32 * scale).round() as u32,
                        ));
                        // This resizes the baseview child view (logical
                        // units) to match the host-resized parent.
                        egui_ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::Vec2::new(
                            new_size.0 as f32,
                            new_size.1 as f32,
                        )));
                        egui_state.size.store(new_size);
                    }
                    egui_state.requested_size.store(None);
                }

                let mut shared = state.shared.lock();
                let shared = &mut *shared;
                let now = shared.start.elapsed().as_secs_f64();

                // Drain note events from the audio thread. Events are
                // re-stamped with the GUI clock on arrival: sub-frame
                // accuracy doesn't matter visually, and it keeps the decay
                // clock and event clock trivially consistent.
                // TODO: estimate the audio→GUI clock offset instead, so
                // event *spacing* within a frame is preserved.
                while let Ok(mut event) = shared.consumer.pop() {
                    event.time = now;
                    shared.ui.tracker.handle_event(event);
                }

                let backend = PluginParamBackend { params: &state.params, setter: &setter };
                lattice_ui::root_ui(egui_ctx, &mut shared.ui, &backend, now);

                resize_corner(egui_ctx, &egui_state, &mut shared.ui.console);
            },
        );

        self.egui_state.open.store(true, Ordering::Release);
        Box::new(LatticeEditorHandle { egui_state: self.egui_state.clone(), window })
    }

    fn size(&self) -> (u32, u32) {
        // If a resize was requested but not yet applied, report the
        // requested size so the host resizes to it.
        self.egui_state
            .requested_size
            .load()
            .unwrap_or_else(|| self.egui_state.size())
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

/// A drag handle in the bottom-right corner that requests a window resize
/// from the host (replaces v1's resize hack; the host round-trip is the
/// sanctioned path).
fn resize_corner(ctx: &Context, egui_state: &EguiState, console: &mut lattice_ui::Console) {
    const CORNER: f32 = 24.0;
    let screen = ctx.content_rect();
    let corner_rect = egui::Rect::from_min_max(
        screen.max - egui::vec2(CORNER, CORNER),
        screen.max,
    );

    egui::Area::new(egui::Id::new("window_resize_corner"))
        .fixed_pos(corner_rect.min)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let response = ui.allocate_response(egui::vec2(CORNER, CORNER), egui::Sense::drag());
            let color = if response.hovered() || response.dragged() {
                ui.visuals().strong_text_color()
            } else {
                ui.visuals().weak_text_color()
            };
            // Diagonal grip lines.
            let painter = ui.painter();
            let max = corner_rect.max;
            for i in 1..=3 {
                let offset = i as f32 * 4.0;
                painter.line_segment(
                    [max - egui::vec2(offset, 2.0), max - egui::vec2(2.0, offset)],
                    egui::Stroke::new(1.5, color),
                );
            }

            // Resize by drag *delta* from the size at drag start, not by
            // absolute pointer position: grabbing the handle anywhere but
            // its exact corner must not snap the window to the pointer.
            let anchor_id = egui::Id::new("window_resize_anchor");
            if response.drag_started() {
                if let Some(pointer) = response.interact_pointer_pos() {
                    let (w, h) = egui_state.size();
                    ctx.data_mut(|d| d.insert_temp(anchor_id, (pointer, w, h)));
                    console.log(format!(
                        "resize drag start at ({:.0},{:.0}), size {}x{}",
                        pointer.x, pointer.y, w, h
                    ));
                }
            }

            if response.dragged() || response.drag_stopped() {
                let anchor: Option<(egui::Pos2, u32, u32)> = ctx.data(|d| d.get_temp(anchor_id));
                if let (Some(pointer), Some((start_pointer, start_w, start_h))) =
                    (response.interact_pointer_pos(), anchor)
                {
                    let width =
                        ((start_w as f32 + (pointer.x - start_pointer.x)).max(400.0)).round() as u32;
                    let height =
                        ((start_h as f32 + (pointer.y - start_pointer.y)).max(300.0)).round() as u32;

                    // Host resizes are asynchronous, so every live request
                    // causes a transient parent/child mismatch (visible
                    // jitter). Throttle mid-drag requests, but always send
                    // a final one on release so the window settles exactly
                    // where the drag ended.
                    const MIN_REQUEST_INTERVAL: f64 = 0.08;
                    let now = ctx.input(|i| i.time);
                    let last_id = egui::Id::new("window_resize_last_request");
                    let last: f64 = ctx.data(|d| d.get_temp(last_id)).unwrap_or(f64::NEG_INFINITY);
                    let due = response.drag_stopped() || now - last >= MIN_REQUEST_INTERVAL;

                    if due && (width, height) != egui_state.size() {
                        egui_state.requested_size.store(Some((width, height)));
                        ctx.data_mut(|d| d.insert_temp(last_id, now));
                    }
                }
                if response.drag_stopped() {
                    console.log("resize drag stop");
                }
            }
        });
}

struct LatticeEditorHandle {
    egui_state: Arc<EguiState>,
    window: WindowHandle,
}

// WindowHandle contains raw pointers, but the handle is only used to close
// the window from the host's GUI thread.
unsafe impl Send for LatticeEditorHandle {}

impl Drop for LatticeEditorHandle {
    fn drop(&mut self) {
        self.egui_state.open.store(false, Ordering::Release);
        self.window.close();
    }
}
