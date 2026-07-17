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
            |_egui_ctx: &Context, _queue, _state: &mut WindowState| {},
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
                lattice_ui::root_ui(ui, &mut shared.ui, &backend, now);

                if SHOW_RESIZE_CORNER {
                    resize_corner(egui_ctx, &egui_state, &mut shared.ui.console);
                }
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

/// Whether to show the in-window resize drag handle. Disabled for now:
/// hosts with proper resize support (Bitwig at least) provide a native
/// window border, which supersedes it. Kept as a fallback in case some
/// host turns out not to honor `ResizeHint::resizable()` — flip this on
/// and the full preview + resize-on-release path comes back.
const SHOW_RESIZE_CORNER: bool = false;

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

                    if response.drag_stopped() {
                        // The one and only host round-trip, on release.
                        if (width, height) != egui_state.size() {
                            egui_state.requested_size.store(Some((width, height)));
                        }
                        console.log(format!("resize drag stop -> {}x{}", width, height));
                    } else {
                        // Mid-drag: preview only, no host round-trips.
                        // Live-resizing the window during the drag makes
                        // some hosts (observed in Bitwig/macOS) break
                        // AppKit's drag capture when they apply the resize:
                        // pointer updates stop for the rest of the drag and
                        // the window freezes until release. One resize on
                        // release also eliminates live-resize flicker.
                        draw_resize_preview(ui, pointer, width, height);
                    }
                }
            }
        });
}

/// Ghost outline + size readout shown while dragging the resize corner.
/// Drawn in window coordinates: when shrinking you see the target outline;
/// when growing (target extends past the current window) the readout near
/// the pointer carries the information.
fn draw_resize_preview(ui: &egui::Ui, pointer: egui::Pos2, width: u32, height: u32) {
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("window_resize_preview"),
    ));
    let visuals = ui.visuals();
    let color = visuals.selection.stroke.color;

    let target = egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(width as f32, height as f32),
    );
    painter.rect_stroke(
        target,
        egui::CornerRadius::same(2),
        egui::Stroke::new(2.0, color),
        egui::StrokeKind::Inside,
    );

    let label_pos = pointer - egui::vec2(16.0, 16.0);
    let galley = painter.layout_no_wrap(
        format!("{} × {}", width, height),
        egui::FontId::proportional(14.0),
        color,
    );
    let bg = egui::Rect::from_min_size(
        label_pos - egui::vec2(galley.size().x, galley.size().y),
        galley.size(),
    )
    .expand(4.0);
    painter.rect_filled(bg, egui::CornerRadius::same(3), visuals.extreme_bg_color);
    painter.galley(bg.shrink(4.0).min, galley, color);
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
