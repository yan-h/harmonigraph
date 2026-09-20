use std::sync::Arc;
use std::time::Instant;

use baseview::{Size, WindowHandle, WindowScalePolicy};
use crossbeam::atomic::AtomicCell;
use egui::Context;
use egui_baseview::{EguiWindow, EguiWindowSettings, GraphicsConfig, SizeSource};
use nice_plug::prelude::{Editor, GuiContext, ParentWindowHandle, ResizeHint};
use parking_lot::Mutex;
use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};

use crate::HarmonigraphParams;

use super::frame::{frame, WindowState};
use super::persist::EguiState;
use super::shared::EditorShared;
use super::MIN_SIZE;

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
    harmonigraph_render::shader_assets::initialize();
    use egui_baseview::WgpuSetup;
    use harmonigraph_render::wgpu;

    let mut config = GraphicsConfig::default();
    if let WgpuSetup::CreateNew(setup) = &mut config.wgpu_options.wgpu_setup {
        let base = setup.device_descriptor.clone();
        setup.device_descriptor = std::sync::Arc::new(move |adapter: &wgpu::Adapter| {
            let mut descriptor = base(adapter);
            descriptor.required_features |= adapter.features() & wgpu::Features::TIMESTAMP_QUERY;
            descriptor
        });
    }
    config
}

pub fn create(
    params: Arc<HarmonigraphParams>,
    shared: Arc<Mutex<EditorShared>>,
) -> Option<Box<dyn Editor>> {
    Some(Box::new(LatticeEditor {
        egui_state: params.editor_state.clone(),
        params,
        shared,
        gpu_context: egui_baseview::SharedGpuContext::default(),
        // On macOS the system reports scaling; elsewhere a host that never
        // calls set_scale_factor gets 1.0 (same policy as nih_plug_egui).
        #[cfg(target_os = "macos")]
        scaling_factor: AtomicCell::new(None),
        #[cfg(not(target_os = "macos"))]
        scaling_factor: AtomicCell::new(Some(1.0)),
    }))
}

pub(super) struct LatticeEditor {
    egui_state: Arc<EguiState>,
    params: Arc<HarmonigraphParams>,
    shared: Arc<Mutex<EditorShared>>,
    scaling_factor: AtomicCell<Option<f32>>,
    gpu_context: egui_baseview::SharedGpuContext,
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
                    shared.ui.picture.runtime.console.log(format!(
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
                shared.ui.picture.runtime.console.log(format!("host resize {width}x{height}"));
            }
            Some(Size::new(f64::from(width), f64::from(height)))
        });

        let mut graphics = graphics_config();
        graphics.shared_context = Some(self.gpu_context.clone());
        let window = EguiWindow::open_parented(
            &ParentWindowHandleAdapter(parent),
            EguiWindowSettings::new()
                .with_logical_size(Size::new(f64::from(unscaled_width), f64::from(unscaled_height)))
                .with_scale_policy(
                    scaling_factor
                        .map(|factor| WindowScalePolicy::ScaleFactor(f64::from(factor)))
                        .unwrap_or(WindowScalePolicy::SystemScaleFactor),
                )
                .with_graphics_config(graphics)
                .with_size_source(size_source),
            WindowState::new(self.shared.clone(), self.params.clone()),
            |egui_ctx: &Context, _queue, state: &mut WindowState| {
                // egui-baseview installs the current font texture in
                // CallbackResources after applying this frame's deltas.
                harmonigraph_ui::use_renderer_font_texture(egui_ctx);
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
                harmonigraph_ui::begin_editor_loading(egui_ctx);
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
        self.egui_state.requested_size.load().unwrap_or_else(|| self.egui_state.size())
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
