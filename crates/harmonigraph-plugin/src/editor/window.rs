use std::sync::Arc;
use std::time::Instant;

use baseview::{Size, WindowHandle, WindowScalePolicy};
use crossbeam::atomic::AtomicCell;
use egui::Context;
use egui_baseview::{EguiWindow, EguiWindowSettings, GraphicsConfig, SizeSource};
use nice_plug::prelude::{Editor, GuiContext, ParentWindowHandle, ResizeHint};
use nice_plug::wrapper::clap::EditorHost;
use parking_lot::Mutex;
use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};

use crate::HarmonigraphParams;

use super::frame::{frame, WindowState};
use super::persist::EguiState;
use super::pop_out::{PopOut, Sized};
use super::shared::EditorShared;
use super::MIN_SIZE;

/// The wgpu setup, which is `GraphicsConfig::default()` with the device
/// replaced by [`harmonigraph_render::device_descriptor`], the one every shell
/// and test uses. egui's own default contributes nothing that one lacks: a
/// label, and limits it already replaces with the adapter's.
///
/// The surface keeps egui-wgpu's `desired_maximum_frame_latency` of 2, which
/// on Metal is already three drawables: wgpu clamps the latency to 1..=2 and
/// wgpu-hal sets `maximumDrawableCount` to latency + 1. So `Some(3)` changes
/// nothing (an earlier note here reported trying it, and tested a no-op), and
/// the only other setting, `Some(1)`, gives two drawables: a frame less
/// latency, bought with a missed refresh whenever a frame runs long. The
/// choice is smoothness (#1551).
pub(super) fn graphics_config() -> GraphicsConfig {
    harmonigraph_render::shader_assets::initialize();
    use egui_baseview::WgpuSetup;

    let mut config = GraphicsConfig::default();
    if let WgpuSetup::CreateNew(setup) = &mut config.wgpu_options.wgpu_setup {
        setup.device_descriptor = std::sync::Arc::new(harmonigraph_render::device_descriptor);
    }
    config
}

pub fn create(
    params: Arc<HarmonigraphParams>,
    shared: Arc<Mutex<EditorShared>>,
    host: Option<Arc<dyn EditorHost>>,
) -> Option<Box<dyn Editor>> {
    Some(Box::new(LatticeEditor {
        egui_state: params.editor_state.clone(),
        params,
        shared,
        host,
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
    host: Option<Arc<dyn EditorHost>>,
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
        let pop_out = PopOut::new();
        let popping = pop_out.clone();
        let size_source = SizeSource::new(move || {
            // The editor's own window resized (see `PopOut`): the editor
            // takes its size, and the project saves it as the editor's.
            let as_size =
                |(width, height): (u32, u32)| Some(Size::new(f64::from(width), f64::from(height)));
            match popping.take_size() {
                Some(Sized::Own(size)) => {
                    resized.size.store(size);
                    return as_size(size);
                }
                // Back in the host's window, which is asked for the editor's
                // size just below, as a fold's ask is.
                Some(Sized::Docked) => resized.requested_size.store(Some(resized.size())),
                None => {}
            }
            // OUR ask first, and from here rather than from inside the frame,
            // for the same reason the host's is collected here: the round trip
            // is synchronous, so asking now means the frame about to be built
            // is laid out at the size it asked for instead of the one it is
            // leaving. Asked from inside the frame, the answer arrives after
            // the layout that wanted it and the arrangement wears an extra
            // frame stretched across the old window.
            if let Some(size) = resized.requested_size.load() {
                // In its own window, the ask is that window's to grant.
                if popping.resize(size) {
                    resized.requested_size.store(None);
                    resized.size.store(size);
                    return as_size(size);
                }
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
            // The host's window, concealed, has no say over the editor's size,
            // and what arrives from it is mostly its echo of the editor's own.
            // `set_size` has already stored it as the size the project saves,
            // which is the editor's own window's again.
            if let Some(own) = popping.size() {
                if resized.host_resized.swap(None).is_some() {
                    resized.size.store(own);
                }
                return None;
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
        let closing_context = context.clone();
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
            WindowState::new(self.shared.clone(), self.params.clone(), pop_out.clone()),
            |egui_ctx: &Context, queue, state: &mut WindowState| {
                queue.set_key_capture(egui_baseview::KeyCapture::CaptureShortcuts(
                    harmonigraph_ui::APPEARANCE_SHORTCUTS.to_vec(),
                ));
                // egui-baseview installs the current font texture in
                // CallbackResources after applying this frame's deltas.
                harmonigraph_ui::use_renderer_font_texture(egui_ctx);
                // Everything this context owes the shared UI state, which is
                // NOT new when the context is: the theme, the release of what
                // the closed window left behind, the fold floor, and the
                // layout the host saved with the project (written by a host
                // restore, or on the way out, in `LatticeEditorHandle`'s Drop).
                //
                // Read with the state's lock held, in the order the close and
                // the input worker take the two (`shared`, then the
                // blob): taking the blob also answers a pending restore, and a
                // save on another thread must not see that answered before the
                // state holds it (see `UiState`).
                let mut shared = state.shared.lock();
                let serialized = state.params.ui_state.opening_blob();
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

        pop_out.open(
            parent,
            &window,
            self.egui_state.docked(),
            self.host.clone(),
            self.shared.clone(),
        );
        self.egui_state.set_open(true);
        Box::new(LatticeEditorHandle {
            pop_out,
            egui_state: self.egui_state.clone(),
            shared: self.shared.clone(),
            params: self.params.clone(),
            context: closing_context,
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
    pop_out: Arc<PopOut>,
    egui_state: Arc<EguiState>,
    shared: Arc<Mutex<EditorShared>>,
    params: Arc<HarmonigraphParams>,
    context: Arc<dyn GuiContext>,
    window: WindowHandle,
}

// WindowHandle contains raw pointers, but the handle is only used to close
// the window from the host's GUI thread.
unsafe impl Send for LatticeEditorHandle {}

impl Drop for LatticeEditorHandle {
    fn drop(&mut self) {
        // The editor's own window closes with the editor rather than
        // outliving it empty, and the host's is shown again.
        self.pop_out.close();
        // Store the UI state (layout, camera, view settings) as the blob a
        // host save reads once the window is shut; while it is open, the
        // save serializes the live state instead (see `UiState`). A restore
        // no frame has drawn yet is applied first, so it is not lost here.
        // The lock is taken here with `open` still TRUE, which is what keeps
        // the input worker from adopting the blob this writes, or ageing the
        // picture, until the window is gone — see [`crate::background`] on why
        // that ordering is load-bearing rather than incidental.
        let mut shared = self.shared.lock();
        let setter = nice_plug::prelude::ParamSetter::new(self.context.as_ref());
        for (key, active) in harmonigraph_ui::params::ParamKey::ALL
            .into_iter()
            .zip(shared.gesture.replace([false; harmonigraph_ui::params::ParamKey::ALL.len()]))
        {
            if active {
                setter.end_set_parameter(self.params.param_for(key));
            }
        }
        self.params.ui_state.store_close(&mut shared.ui);
        drop(shared);
        self.egui_state.set_open(false);
        self.window.close();
    }
}
