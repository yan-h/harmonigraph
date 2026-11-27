use std::ffi::c_void;
use std::sync::Arc;
use std::time::Instant;

use egui::Context;
use egui_baseview::baseview::{Size, WindowHandle, WindowScalePolicy};
use egui_baseview::{EguiWindow, EguiWindowSettings, GraphicsConfig, SizeSource};
use harmonigraph_clap::prelude::{Editor, EditorHost};

use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};

use crate::HarmonigraphParams;

use super::frame::{frame, WindowState};
use super::lock::EditorLock;
use super::open_window_session;
use super::persist::EguiState;

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
                // As egui-wgpu asks, so the adapter is the one it would pick.
                apply_limit_buckets: false,
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
    host: Arc<dyn EditorHost>,
) -> Box<dyn Editor> {
    Box::new(LatticeEditor { egui_state: params.editor_state.clone(), params, shared, host })
}

pub(super) struct LatticeEditor {
    egui_state: Arc<EguiState>,
    params: Arc<HarmonigraphParams>,
    shared: Arc<EditorLock>,
    host: Arc<dyn EditorHost>,
}

/// The host's `NSView` as baseview takes a parent.
struct HostView(*mut c_void);

unsafe impl HasRawWindowHandle for HostView {
    fn raw_window_handle(&self) -> RawWindowHandle {
        let mut handle = raw_window_handle::AppKitWindowHandle::empty();
        handle.ns_view = self.0;
        RawWindowHandle::AppKit(handle)
    }
}

impl Editor for LatticeEditor {
    /// # Safety
    ///
    /// `parent` is a live `NSView`, and the call is on the main thread.
    unsafe fn spawn(&self, parent: *mut c_void) -> Box<dyn std::any::Any + Send> {
        let context = self.host.clone();
        let egui_state = self.egui_state.clone();
        // Open before the window, whose size source hands this back.
        let opened = self.egui_state.open();
        let (width, height) = self.egui_state.size();
        // Where the window collects its size before it builds the frame that
        // has to be laid out at it (see `next_size`). A host resizing the
        // parent view reaches the plugin only as `Editor::set_size`, a CLAP
        // `gui` call on the main thread outside this window's callbacks, and
        // baseview raises no event for it on macOS, so the window asks here.
        let size_source = {
            let (state, host, shared) =
                (self.egui_state.clone(), context.clone(), self.shared.clone());
            SizeSource::new(move || {
                let (width, height) = next_size(&state, opened, host.as_ref(), &shared)?;
                Some(Size::new(f64::from(width), f64::from(height)))
            })
        };

        // The editor stopped being seen (covered, minimized, ordered out, on
        // another Space, or its view hidden or moved into a window not yet on
        // screen): no frame runs until it is seen again, so what a frame would
        // end is ended here. An open look edit becomes one undo entry, and
        // every host gesture the editor holds is ended, with the editor lock
        // free (`EditorLock::end_unseen`). baseview calls this on the main
        // thread, outside its handler's borrow; it can come inside this
        // editor's own moves of its view (the own window's open, its deferred
        // moves and its close), none of which is made under the editor lock,
        // and whose own callback into `EditorSize` takes no lock.
        let unseen = {
            let (shared, context) = (self.shared.clone(), context.clone());
            egui_baseview::baseview::UnseenNotice::new(move || {
                let Some(open) = shared.end_unseen() else { return };
                super::gestures::end(open, context.as_ref());
            })
        };
        let mut graphics = graphics_config();
        graphics.shared_context = Some(self.shared.lock().gpu_context.clone());
        let closing_context = context.clone();
        let window = EguiWindow::open_parented(
            &HostView(parent),
            EguiWindowSettings::new()
                .with_logical_size(Size::new(f64::from(width), f64::from(height)))
                .with_scale_policy(WindowScalePolicy::SystemScaleFactor)
                .with_graphics_config(graphics)
                .with_size_source(size_source)
                .with_unseen_notice(unseen)
                .with_pass_request(self.egui_state.pass_request.clone()),
            WindowState::new(self.shared.clone(), self.params.clone()),
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

        // Learn the editor's views, and move it into its own window unless the
        // project keeps it docked. Outside any frame and holding no lock a
        // frame takes, as the move can draw one on the spot. Where baseview
        // opened no AppKit view the own window is never offered.
        if let RawWindowHandle::AppKit(view) = window.raw_window_handle() {
            // The window's close button tells the host, which destroys the
            // editor.
            let host = self.host.clone();
            let closed: Box<dyn Fn() -> bool> = Box::new(move || host.closed());
            // SAFETY: the host's view the editor opened in, and baseview's
            // inside it, both alive for the call, on the main thread.
            unsafe {
                self.egui_state.window.attach(
                    parent,
                    view.ns_view,
                    !self.egui_state.docked(),
                    Some(closed),
                )
            };
        }
        self.egui_state.settle();
        let handle = LatticeEditorHandle {
            egui_state: self.egui_state.clone(),
            shared: self.shared.clone(),
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

    /// The size the host is told: a proposal while the host is being asked
    /// for it, and the accepted size otherwise.
    fn size(&self) -> (u32, u32) {
        self.egui_state.window.reported_size()
    }

    // Hosts with proper resize support (Bitwig at least) provide a native
    // window border on this hint. An in-window drag-corner fallback for
    // hosts that ignore it existed until July 2026 — recover it from git
    // history (`resize_corner`, in `midi_lattice_3d/src/editor.rs` until
    // 28ce5912) if such a host turns up.
    fn resizable(&self) -> bool {
        true
    }

    /// A size the host's border drag proposes, held to the editor's limits,
    /// so the drag stops at them.
    fn adjust_size(&self, width: u32, height: u32) -> (u32, u32) {
        self.egui_state.window.limits().clamp((width, height))
    }

    /// Refused outside the limits, and otherwise taken, unless the own window
    /// has the say over the size: the view is in it, or, before the host
    /// attaches the editor, the project will open it there
    /// ([`EditorSize::host_set_size`](plugin_own_window::EditorSize::host_set_size)).
    fn set_size(&self, width: u32, height: u32) -> bool {
        let state = &self.egui_state.window;
        let size = (width, height);
        // Its own proposal echoed back, or the size it has, is no news.
        let told = state.reported_size();
        let taken = state.host_set_size(size, || !self.egui_state.docked());
        let line = if !taken {
            Some(format!("host resize {width}x{height} REFUSED: outside the editor's limits"))
        } else {
            (size != told && state.size() == size).then(|| format!("host resize {width}x{height}"))
        };
        // try_lock, and only for a line: the host may size the editor while a
        // frame holds the lock, and the main thread must not wait for itself.
        if let Some(line) = line {
            if let Some(mut shared) = self.shared.try_lock() {
                shared.ui.picture.runtime.console.log(line);
            }
        }
        taken
    }
}

/// The frame's size, from the size source of the open editor `opened`, asked
/// before each frame's input and layout
/// ([`EditorSize::next_frame`](plugin_own_window::EditorSize::next_frame)):
/// the own window's, then a size the editor or a restore asked for, which the
/// own window grants or `host` is asked for, then one the host set. `None`
/// when the frame keeps the size it had.
///
/// Asked here rather than from inside the frame, because the round trip is
/// synchronous: the frame about to be built is laid out at the size it asked
/// for, rather than wearing an extra frame stretched across the old window.
/// Each request to the host leaves a Console line, written once the host has
/// answered, as no lock is held across the call.
pub(crate) fn next_size(
    state: &EguiState,
    opened: plugin_own_window::Opened,
    host: &dyn EditorHost,
    shared: &EditorLock,
) -> Option<(u32, u32)> {
    state.window.next_frame(opened, || {
        let (width, height) = state.window.reported_size();
        let started = Instant::now();
        let accepted = host.request_resize((width, height));
        let roundtrip = started.elapsed().as_secs_f64() * 1000.0;
        if let Some(mut shared) = shared.try_lock() {
            shared.ui.picture.runtime.console.log(format!(
                "request_resize {width}x{height} -> {} ({roundtrip:.1} ms)",
                if accepted { "accepted" } else { "REFUSED" },
            ));
        }
        accepted
    })
}

struct LatticeEditorHandle {
    egui_state: Arc<EguiState>,
    shared: Arc<EditorLock>,
    context: Arc<dyn EditorHost>,
    window: WindowHandle,
}

// Off macOS, the shell still closes this raw-pointer handle from the host's
// GUI thread. On macOS the returned MainThreadBound owns that guarantee.
#[cfg(not(target_os = "macos"))]
unsafe impl Send for LatticeEditorHandle {}

impl Drop for LatticeEditorHandle {
    fn drop(&mut self) {
        // The session ends under the lock, and the host's gestures once it is
        // let go (see `EditorLock::end_window_session`). The
        // input worker ages the picture whether or not the window is open, so
        // where `open` is cleared below sets only its poll rate and whether
        // the tape treats its roll as drawn (`tape::Worker::exchange`).
        let open = self.shared.end_window_session();
        super::gestures::end(open, self.context.as_ref());
        // The editor's own window closes with the editor rather than
        // outliving it empty, and the host's is shown again. With the editor
        // lock free: closing can hide the view, which the unseen notice
        // hears, and draw a frame on the spot.
        self.egui_state.close();
        self.window.close();
    }
}

#[cfg(test)]
mod tests {
    use harmonigraph_render::startup::{Status, WindowStartup};

    /// What activation does (#1746): the device it makes is the one the
    /// window will reuse AND the one the job compiles on, and activating again
    /// makes no second device and moves the job to none. Under hot reload,
    /// which the workspace's test run turns on, the device half still holds;
    /// only the job is missing, since that build initializes synchronously.
    /// So the job half runs only without hot reload, in
    /// `cargo test -p harmonigraph-plugin`; `ci.sh`'s workspace gate reaches
    /// the device half and the early return below it.
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
