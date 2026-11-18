//! A native parented editor without a DAW. Uses the production graphics setup,
//! loading path and frame pacing, and closes children from their host callback.
//! Times are observed on the update after presentation, not display photons.
//!
//! Its window must be on screen, uncovered, for the whole run: every opening
//! and close is sequenced from its window's frames, and a window that cannot
//! be seen runs none. A run that stalls so is ended by [`TIMEOUT`]
//! from a thread of its own, which needs no frame.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use baseview::{Event, EventStatus, Window, WindowHandle, WindowHandler, WindowOpenOptions};
use egui_baseview::{EguiWindow, EguiWindowSettings, GraphicsConfig, SharedGpuContext, WgpuSetup};
use harmonigraph_ui::{
    params::{ParamBackend, ParamKey},
    SharedState,
};
use parking_lot::Mutex;

const OPENINGS: usize = 3;

/// How long the whole run may take before the probe gives up and exits.
const TIMEOUT: Duration = Duration::from_secs(90);

struct Defaults;
impl ParamBackend for Defaults {
    fn get(&self, key: ParamKey) -> f32 {
        key.default_value()
    }
    fn set(&self, _: ParamKey, _: f32) {}
}

#[derive(Default)]
struct Observations {
    completed: AtomicUsize,
    devices: AtomicUsize,
    dropped: AtomicUsize,
}

struct Host {
    shared: Arc<Mutex<SharedState>>,
    context: SharedGpuContext,
    observations: Arc<Observations>,
    child: Option<WindowHandle>,
    opening: usize,
}

/// The editor's graphics setup, counting every device made from it.
fn counted(observations: &Arc<Observations>) -> GraphicsConfig {
    let mut graphics = super::graphics_config();
    if let WgpuSetup::CreateNew(setup) = &mut graphics.wgpu_options.wgpu_setup {
        let base = setup.device_descriptor.clone();
        let observations = observations.clone();
        setup.device_descriptor = Arc::new(move |adapter| {
            observations.devices.fetch_add(1, Ordering::Relaxed);
            base(adapter)
        });
    }
    graphics
}

/// Open and close three real parented editors with one retained device, made
/// as activation makes it, before the first opens: the count of one says no
/// window made another.
pub fn run() {
    let shared = Arc::new(Mutex::new(SharedState::new(super::ASSUMED_SURFACE_FORMAT)));
    let observations = Arc::new(Observations::default());
    let context = SharedGpuContext::default();
    {
        let state = shared.lock();
        super::start_graphics(
            &context,
            &state.picture.editor_graphics(),
            &counted(&observations),
            state.picture.surfaces.target_format,
        );
    }
    assert_eq!(observations.devices.load(Ordering::Relaxed), 1, "activation made no device");
    // Ended when `run` returns, which drops the sender.
    let (finished, timer) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        if let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = timer.recv_timeout(TIMEOUT) {
            eprintln!(
                "NATIVE_LIFECYCLE timed out after {}s: is the probe's window on screen?",
                TIMEOUT.as_secs()
            );
            std::process::exit(1);
        }
    });
    let host_shared = shared.clone();
    let host_observations = observations.clone();
    Window::open_blocking(
        WindowOpenOptions::new()
            .with_title("Harmonigraph startup measurement")
            .with_size(1000.0, 700.0),
        move |_| {
            // A command-line application can start behind another window,
            // and Metal skips presentation while it is occluded. Brought
            // forward as the window is built rather than from its first
            // frame, which a window that cannot be seen never runs.
            use objc2::MainThreadMarker;
            let app =
                objc2_app_kit::NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
            Host {
                shared: host_shared,
                context,
                observations: host_observations,
                child: None,
                opening: 0,
            }
        },
    );
    shared.lock().picture.editor_graphics().shutdown_startup();
    assert_eq!(observations.completed.load(Ordering::Relaxed), OPENINGS);
    assert_eq!(observations.dropped.load(Ordering::Relaxed), OPENINGS);
    assert_eq!(observations.devices.load(Ordering::Relaxed), 1, "reopen recreated device");
    eprintln!("NATIVE_LIFECYCLE openings={OPENINGS} devices=1 dropped={OPENINGS}");
    drop(finished);
}

impl WindowHandler for Host {
    fn on_frame(&mut self, window: &mut Window) {
        if self.child.is_some() {
            if self.observations.completed.load(Ordering::Relaxed) > self.opening {
                self.child.take().unwrap().close();
                self.opening += 1;
            }
            return;
        }
        if self.opening == OPENINGS {
            self.shared.lock().picture.editor_graphics().shutdown_startup();
            window.close();
            // Wake NSApplication after baseview calls stop() from a frame.
            use objc2::MainThreadMarker;
            use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType};
            use objc2_foundation::NSPoint;
            let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
            let event = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
                NSEventType::ApplicationDefined, NSPoint::ZERO, NSEventModifierFlags::empty(),
                0.0, 0, None, 0, 0, 0,
            );
            if let Some(event) = event {
                app.postEvent_atStart(&event, true);
            }
            return;
        }
        let mut graphics = counted(&self.observations);
        graphics.shared_context = Some(self.context.clone());
        self.child = Some(EguiWindow::open_parented(
            window,
            EguiWindowSettings::new()
                .with_logical_size(baseview::Size::new(1000.0, 700.0))
                .with_graphics_config(graphics),
            Probe {
                shared: self.shared.clone(),
                observations: self.observations.clone(),
                opening: self.opening,
                opened: Instant::now(),
                first: false,
                full: false,
                painted: Arc::new(AtomicBool::new(false)),
                ready: Arc::new(AtomicBool::new(false)),
            },
            move |ctx, _, probe: &mut Probe| {
                harmonigraph_ui::shell::Opening { ctx, state: &mut probe.shared.lock() }.open();
                harmonigraph_ui::begin_editor_loading(ctx);
            },
            |ui, queue, probe: &mut Probe| {
                if queue.render_ms() > 0.0 {
                    if !probe.first && probe.painted.load(Ordering::Relaxed) {
                        probe.first = true;
                        eprintln!(
                            "NATIVE_FRAME opening={} stage=first elapsed_ms={:.3}",
                            probe.opening,
                            probe.opened.elapsed().as_secs_f64() * 1000.0
                        );
                    }
                    if !probe.full && probe.ready.load(Ordering::Relaxed) {
                        probe.full = true;
                        probe.observations.completed.fetch_add(1, Ordering::Relaxed);
                        eprintln!(
                            "NATIVE_FRAME opening={} stage=full elapsed_ms={:.3}",
                            probe.opening,
                            probe.opened.elapsed().as_secs_f64() * 1000.0
                        );
                    }
                }
                let mut shared = probe.shared.lock();
                let requested = harmonigraph_ui::shell::Frame {
                    ui,
                    state: &mut shared,
                    params: &Defaults,
                    now: probe.opened.elapsed().as_secs_f64(),
                    build_tag: crate::BUILD_TAG,
                    window_size: egui::vec2(1000.0, 700.0),
                }
                .draw();
                assert!(requested.is_none(), "startup fixture unexpectedly resized");
                queue.set_frame_divisor(shared.workspace.interaction.prefs.frame_rate.divisor());
                let ready = harmonigraph_ui::editor_loading_status(ui.ctx()).is_none();
                egui::Area::new(egui::Id::new("startup-measurement-marker"))
                    .fixed_pos(egui::pos2(10.0, 10.0))
                    .order(egui::Order::Tooltip)
                    .show(ui.ctx(), |ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                            rect,
                            Marker(probe.painted.clone(), ready.then(|| probe.ready.clone())),
                        ));
                    });
                ui.ctx().request_repaint();
            },
        ));
    }

    fn on_event(&mut self, _: &mut Window, _: Event) -> EventStatus {
        EventStatus::Ignored
    }
}

struct Probe {
    shared: Arc<Mutex<SharedState>>,
    observations: Arc<Observations>,
    opening: usize,
    opened: Instant,
    first: bool,
    full: bool,
    painted: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
}

impl Drop for Probe {
    fn drop(&mut self) {
        self.observations.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

struct Marker(Arc<AtomicBool>, Option<Arc<AtomicBool>>);
impl egui_wgpu::CallbackTrait for Marker {
    fn paint(
        &self,
        _: egui::PaintCallbackInfo,
        _: &mut harmonigraph_render::wgpu::RenderPass<'static>,
        _: &egui_wgpu::CallbackResources,
    ) {
        self.0.store(true, Ordering::Relaxed);
        if let Some(ready) = &self.1 {
            ready.store(true, Ordering::Relaxed);
        }
    }
}
