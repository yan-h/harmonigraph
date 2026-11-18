use std::sync::Arc;
use std::time::Instant;

use baseview::{Size, WindowHandle, WindowScalePolicy};
use crossbeam::atomic::AtomicCell;
use egui::Context;
use egui_baseview::{EguiWindow, EguiWindowSettings, GraphicsConfig, SizeSource};
use nice_plug::prelude::{Editor, GuiContext, ParentWindowHandle, ResizeHint};
use nice_plug::wrapper::clap::EditorHost;

use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};

use crate::HarmonigraphParams;

use super::frame::{frame, WindowState};
use super::lock::EditorLock;
use super::open_window_session;
use super::persist::EguiState;
use super::pop_out::{PopOut, Sized};
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
pub(crate) fn graphics_config() -> GraphicsConfig {
    use egui_baseview::WgpuSetup;

    let mut config = GraphicsConfig::default();
    if let WgpuSetup::CreateNew(setup) = &mut config.wgpu_options.wgpu_setup {
        setup.device_descriptor = std::sync::Arc::new(harmonigraph_render::device_descriptor);
    }
    config
}

/// Make the device the editor will open on, retain it in `context`, and start
/// compiling `format`'s programs on it in `cache`, so the first open finds the
/// loading job done or under way rather than starting it cold (#1746).
///
/// The device is made from `config`, the window's own setup, and on the
/// instance egui-baseview's renderer makes when it has none retained, so the
/// window reuses it rather than making another; a window whose surface this
/// adapter cannot serve still makes its own. A device already retained, by an
/// earlier call or a window, is reused, so a later call starts nothing new
/// unless that device was lost.
pub(crate) fn start_graphics(
    context: &egui_baseview::SharedGpuContext,
    cache: &harmonigraph_render::LatticePipelineCache,
    config: &GraphicsConfig,
    format: harmonigraph_render::wgpu::TextureFormat,
) {
    use harmonigraph_render::wgpu;
    let setup = match context.existing() {
        Some(setup) => setup,
        None => {
            let egui_baseview::WgpuSetup::CreateNew(create) = &config.wgpu_options.wgpu_setup
            else {
                eprintln!(
                    "Harmonigraph: no graphics head start; the editor's setup creates no device"
                );
                return;
            };
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let options = wgpu::RequestAdapterOptions {
                power_preference: create.power_preference,
                compatible_surface: None,
                force_fallback_adapter: false,
            };
            let adapter = match pollster::block_on(instance.request_adapter(&options)) {
                Ok(adapter) => adapter,
                Err(error) => {
                    eprintln!("Harmonigraph: no graphics head start; no adapter: {error}");
                    return;
                }
            };
            let descriptor = (create.device_descriptor)(&adapter);
            let (device, queue) = match pollster::block_on(adapter.request_device(&descriptor)) {
                Ok(pair) => pair,
                Err(error) => {
                    eprintln!("Harmonigraph: no graphics head start; no device: {error}");
                    return;
                }
            };
            let setup = egui_wgpu::WgpuSetupExisting { instance, adapter, device, queue };
            context.retain(setup.clone());
            setup
        }
    };
    cache.start(&setup.instance, &setup.device, format, config.renderer_options);
}

pub fn create(
    params: Arc<HarmonigraphParams>,
    shared: Arc<EditorLock>,
    host: Option<Arc<dyn EditorHost>>,
) -> Option<Box<dyn Editor>> {
    Some(Box::new(LatticeEditor {
        egui_state: params.editor_state.clone(),
        params,
        shared,
        host,
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
    shared: Arc<EditorLock>,
    host: Option<Arc<dyn EditorHost>>,
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
        // reaches the plugin only as `Editor::set_size`, a CLAP `gui` call and
        // so on the main thread, outside this window's callbacks.
        let resized = self.egui_state.clone();
        let logging = self.shared.clone();
        let asking = context.clone();
        let pop_out = PopOut::new(self.egui_state.clone());
        let popping = Arc::downgrade(&pop_out);
        let size_source = SizeSource::new(move || {
            // The editor handle owns the controller. A retained view cannot
            // keep its native owner alive through this callback.
            let popping = popping.upgrade()?;
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
        graphics.shared_context = Some(self.shared.lock().gpu_context.clone());
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
                .with_size_source(size_source)
                .with_pass_request(self.egui_state.pass_request.clone()),
            WindowState::new(self.shared.clone(), self.params.clone(), Arc::downgrade(&pop_out)),
            |egui_ctx: &Context, _queue, state: &mut WindowState| {
                // Everything this context owes the shared UI state, which is
                // NOT new when the context is: the theme, the fold floor and
                // the new window session. The settings are already in the state; a
                // host restore waiting is installed by the lock.
                open_window_session(&state.shared, egui_ctx);
            },
            // Thin shim: the real per-frame work is `frame`, above. The
            // closure exists only to own `egui_state`/`context` for the
            // window's lifetime.
            move |ui: &mut egui::Ui, queue, state: &mut WindowState| {
                frame(ui, queue, state, &egui_state, context.as_ref());
            },
        );

        pop_out.open(parent, &window, self.egui_state.docked(), self.host.clone());
        self.egui_state.set_open(true);
        let handle = LatticeEditorHandle {
            pop_out,
            egui_state: self.egui_state.clone(),
            shared: self.shared.clone(),
            params: self.params.clone(),
            context: closing_context,
            window,
        };
        // Bind the entire teardown, including the session's end and baseview's
        // non-Send handle, rather than only its native controller. Normal
        // destruction is on main; an off-main drop synchronously waits for it.
        #[cfg(target_os = "macos")]
        let handle = dispatch2::MainThreadBound::new(
            handle,
            objc2::MainThreadMarker::new().expect("the editor is created on the main thread"),
        );
        Box::new(handle)
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
        // In its own window the editor's size is that window's, which the
        // project saves, and the host's window, concealed, has no say over
        // it: what arrives from there is mostly its echo of the editor's own.
        // Stored, it would be what a save keeps until the own window next
        // resized, however long the editor went unseen. Accepted all the
        // same, as the host's window may be sized as the host likes. CLAP
        // makes this call on the main thread, where `PopOut` moves the view
        // and sets the flag, so the two cannot interleave.
        if self.egui_state.in_own_window() {
            return true;
        }
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
        // root_ui repaints while animating and polls otherwise; nothing to do.
    }

    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {}

    fn param_values_changed(&self) {}
}

struct LatticeEditorHandle {
    // The sole persistent owner; view callbacks upgrade weak references only
    // for their invocation, so retaining the view cannot retain this owner.
    pop_out: Arc<PopOut>,
    egui_state: Arc<EguiState>,
    shared: Arc<EditorLock>,
    params: Arc<HarmonigraphParams>,
    context: Arc<dyn GuiContext>,
    window: WindowHandle,
}

// Off macOS, nice-plug still closes this raw-pointer handle from the host's
// GUI thread. On macOS the returned MainThreadBound owns that guarantee.
#[cfg(not(target_os = "macos"))]
unsafe impl Send for LatticeEditorHandle {}

impl Drop for LatticeEditorHandle {
    fn drop(&mut self) {
        // The session ends, and the host's gestures with it, under the lock
        // (see `EditorLock::end_window_session` on why in that order). The
        // input worker ages the picture whether or not the window is open, so
        // where `open` is cleared below sets only its poll rate and whether
        // the tape treats its roll as drawn (`tape::Worker::exchange`).
        let setter = nice_plug::prelude::ParamSetter::new(self.context.as_ref());
        self.shared.end_window_session(|gestures| {
            for (key, active) in harmonigraph_ui::params::ParamKey::ALL.into_iter().zip(gestures) {
                if active {
                    setter.end_set_parameter(self.params.param_for(key));
                }
            }
        });
        // The editor's own window closes with the editor rather than
        // outliving it empty, and the host's is shown again.
        self.pop_out.close();
        self.egui_state.set_open(false);
        self.window.close();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use harmonigraph_render::startup::{Status, WindowStartup};

    /// A host resize while the editor sits in its own window leaves the size
    /// the project saves, which is that window's; in the host's window it is
    /// the editor's size.
    #[test]
    fn a_host_resize_leaves_the_saved_size_while_the_editor_is_in_its_own_window() {
        let (_notes, consumer) = harmonigraph_record::publication::channel();
        let (_audio, audio_consumer) = crate::audio_ingress::channel(64);
        let (_recorder, control) = harmonigraph_record::channel();
        let shared = super::super::EditorShared::new(
            consumer,
            audio_consumer,
            Arc::new(std::sync::atomic::AtomicU32::new(48_000.0f32.to_bits())),
            control,
        );
        let params = Arc::new(crate::HarmonigraphParams::default());
        let lock = Arc::new(super::EditorLock::new(shared, Arc::default()));
        let editor = super::create(params.clone(), lock, None).expect("an editor");
        let state = &params.editor_state;
        let saved = state.size();
        let (width, height) = (saved.0 + 100, saved.1 + 50);

        state.set_in_own_window(true);
        assert!(editor.set_size(width, height));
        assert_eq!(state.size(), saved, "the host's size replaced the own window's");

        state.set_in_own_window(false);
        assert!(editor.set_size(width, height));
        assert_eq!(state.size(), (width, height), "docked, the host's resize was refused");
    }

    /// What activation does (#1746): the device it makes is the one the
    /// window will reuse AND the one the job compiles on, and activating again
    /// makes no second device and moves the job to none. Under hot reload,
    /// which the workspace's test run turns on, the device half still holds;
    /// only the job is missing, since that build initializes synchronously.
    #[test]
    fn activation_retains_the_device_its_job_compiles_on() {
        if harmonigraph_render::test_gpu_adapter().is_none() {
            return;
        }
        let context = egui_baseview::SharedGpuContext::default();
        let cache = harmonigraph_render::LatticePipelineCache::default();
        let config = super::graphics_config();
        let format = super::super::ASSUMED_SURFACE_FORMAT;
        super::start_graphics(&context, &cache, &config, format);
        let setup = context.existing().expect("activation retained no device");
        super::start_graphics(&context, &cache, &config, format);
        let again = context.existing().expect("the retained device went");
        assert!(again.device == setup.device, "a second activation made another device");

        if matches!(WindowStartup::default().status(), Status::Ready { .. }) {
            eprintln!("hot reload initializes synchronously; no startup job to check");
            return;
        }
        let (instance, device) = cache.startup_device().expect("activation started no job");
        assert!(instance == setup.instance, "the job runs on another instance");
        assert!(device == setup.device, "the job compiles on another device");
        cache.shutdown_startup();
    }
}
