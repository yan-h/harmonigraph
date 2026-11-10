//! One initialization worker per retained editor cache. Only pipeline construction
//! moves off the window thread; each window still owns fresh mutable resources.

use std::sync::{Arc, Mutex};

use crate::{wgpu, CallbackResources, CallbackTrait, LatticePipelineCache, ScreenDescriptor};

/// Coarse construction boundaries, rather than an estimate of time remaining.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    #[default]
    Graphics,
    Shapes,
    Lighting,
    Bloom,
    Lattice,
    Labels,
    Shadows,
    Interface,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Graphics => "Preparing graphics…",
            Self::Shapes => "Preparing shapes…",
            Self::Lighting => "Preparing lighting…",
            Self::Bloom => "Preparing bloom…",
            Self::Lattice => "Preparing the lattice…",
            Self::Labels => "Preparing labels…",
            Self::Shadows => "Preparing shadows…",
            Self::Interface => "Preparing the interface…",
        }
    }
}

/// Published after a loading callback has inspected the actual window device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Preparing(Stage),
    /// A fresh build needs one final loading frame before normal UI callbacks run.
    Ready {
        built: bool,
    },
    Failed,
}

/// Window-local progress. The job itself belongs to the retained pipeline cache,
/// so closing/reopening a window does not start another compilation worker.
#[derive(Clone)]
pub struct WindowStartup(Arc<Mutex<Status>>);

impl Default for WindowStartup {
    fn default() -> Self {
        // Development hot reload keeps its existing synchronous resource path.
        let status = if cfg!(feature = "hot-reload") {
            Status::Ready { built: false }
        } else {
            Status::Preparing(Stage::Graphics)
        };
        Self(Arc::new(Mutex::new(status)))
    }
}

impl WindowStartup {
    pub fn status(&self) -> Status {
        *self.0.lock().expect("startup status poisoned")
    }

    /// A preparation-only callback: egui draws the screen using its own pipeline.
    pub fn callback(
        &self,
        rect: egui::Rect,
        cache: Arc<LatticePipelineCache>,
        format: wgpu::TextureFormat,
    ) -> egui::PaintCallback {
        egui_wgpu::Callback::new_paint_callback(
            rect,
            LoadingCallback { state: self.clone(), cache, format },
        )
    }
}

struct LoadingCallback {
    state: WindowStartup,
    cache: Arc<LatticePipelineCache>,
    format: wgpu::TextureFormat,
}

impl CallbackTrait for LoadingCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        #[cfg(not(feature = "hot-reload"))]
        let status = if let Some(instance) = resources.get::<wgpu::Instance>().cloned() {
            let status = self.cache.poll_startup(&instance, device, self.format);
            if matches!(status, Status::Ready { .. }) {
                self.cache.install(&instance, device, queue, self.format, resources);
            }
            status
        } else {
            // Other shells have not opted into the retained-device contract.
            Status::Ready { built: false }
        };
        #[cfg(feature = "hot-reload")]
        let status = {
            let _ = (device, queue, resources, &self.cache, self.format);
            Status::Ready { built: false }
        };
        *self.state.0.lock().expect("startup status poisoned") = status;
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        _pass: &mut wgpu::RenderPass<'static>,
        _resources: &CallbackResources,
    ) {
    }
}

/// What the worker compiles: the lattice's programs, and every other pane's
/// that depends on nothing but the device and the target format. Clones are
/// GPU handles; each window wraps them in fresh buffers and targets of its own.
#[cfg(not(feature = "hot-reload"))]
pub(super) struct Compiled {
    pub(super) lattice: super::CompiledLatticeResources,
    pub(super) panes: PanePipelines,
}

#[cfg(not(feature = "hot-reload"))]
impl Compiled {
    pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        Self::new_with_progress(device, format, |_| {})
    }

    fn new_with_progress(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        progress: impl Fn(Stage),
    ) -> Self {
        let lattice = super::CompiledLatticeResources::new_with_progress(device, format, &progress);
        // The lattice's last step reports Interface already, and these are
        // the interface's own panes, so they need no label of their own.
        Self { lattice, panes: PanePipelines::new(device, format) }
    }
}

/// The roll's, the Spiral's halo and dot cut, the names' and the
/// spectrogram's programs, cloud included. Built here, off the window thread,
/// rather than by the frame that first draws each — the first note, a reopen
/// with notes on the roll — which measured 14-27 ms of the host's main thread
/// once per opening (#1670, R1). Built whether or not this session draws them:
/// the loading screen is where that time costs least.
///
/// Targets sized to a pane stay with the frame that knows the size.
#[cfg(not(feature = "hot-reload"))]
pub(super) struct PanePipelines {
    roll: crate::roll::RollPipelines,
    glow: crate::glow::GlowPipelines,
    dots: crate::dot_surround::Pipelines,
    text: crate::text::TextPipelines,
    spectrogram: crate::spectrogram::SpectrogramPipelines,
}

#[cfg(not(feature = "hot-reload"))]
impl PanePipelines {
    pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        Self {
            roll: crate::roll::RollPipelines::new(device, format),
            glow: crate::glow::GlowPipelines::new(device, format),
            dots: crate::dot_surround::Pipelines::new(device, format),
            text: crate::text::TextPipelines::new(device, format),
            spectrogram: crate::spectrogram::SpectrogramPipelines::new(device, format)
                .with_cloud(device),
        }
    }

    /// Give each pane these programs, unless it already holds its own.
    pub(super) fn adopt(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &mut CallbackResources,
    ) {
        crate::roll::RollResources::adopt(&self.roll, resources);
        crate::glow::GlowResources::adopt(&self.glow, resources);
        crate::dot_surround::Resources::adopt(&self.dots, resources);
        crate::text::TextResources::adopt(&self.text, device, queue, resources);
        crate::spectrogram::SpectrogramResources::adopt(&self.spectrogram, resources);
    }
}

#[cfg(test)]
thread_local! {
    /// The pane programs this thread has compiled, by name. Per thread because
    /// tests run side by side, and the worker compiles on a thread of its own.
    static BUILT: std::cell::RefCell<Vec<&'static str>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

/// Called by each constructor [`PanePipelines`] holds, and so by the frame that
/// builds one where the loading stage did not.
#[cfg(test)]
pub(crate) fn record_build(what: &'static str) {
    BUILT.with(|built| built.borrow_mut().push(what));
}

#[cfg(not(feature = "hot-reload"))]
#[derive(Default)]
pub(super) struct Initialization {
    job: Option<Job>,
    stopped: bool,
}

impl LatticePipelineCache {
    /// Join initialization before unloading plugin code, even if a window still
    /// retains shared UI state. Closing just the editor must not call this.
    pub fn shutdown_startup(&self) {
        #[cfg(not(feature = "hot-reload"))]
        {
            let mut startup = self.startup.lock().expect("startup job poisoned");
            startup.stopped = true;
            drop(startup.job.take());
        }
    }
}

#[cfg(all(test, not(feature = "hot-reload")))]
mod tests {
    use super::*;

    fn take_builds() -> Vec<&'static str> {
        let mut built = BUILT.with(|built| std::mem::take(&mut *built.borrow_mut()));
        built.sort_unstable();
        built
    }

    /// After loading, the frame that first draws a note on the roll with its
    /// cut and bloom, a lit Spiral mark with its cut, a name, and a clouded
    /// spectrogram compiles no pipeline (#1670, R1). The same frame on a
    /// renderer that skipped loading compiles all six, which is what says the
    /// frame reaches every build rather than declining one.
    #[test]
    fn the_first_drawing_frame_after_loading_builds_no_pipeline() {
        const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
        let Some((instance, adapter)) = crate::test_gpu_adapter() else {
            return;
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&crate::device_descriptor(&adapter)))
                .unwrap();
        let screen = ScreenDescriptor { size_in_pixels: [256, 256], pixels_per_point: 1.0 };
        let full = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(256.0, 256.0));
        // Through egui-wgpu's own renderer, as the editor's frames are.
        let frame = |renderer: &mut egui_wgpu::Renderer, callbacks: Vec<egui::PaintCallback>| {
            let primitives: Vec<_> = callbacks
                .into_iter()
                .map(|callback| egui::ClippedPrimitive {
                    clip_rect: full,
                    primitive: egui::epaint::Primitive::Callback(callback),
                })
                .collect();
            let mut encoder = device.create_command_encoder(&Default::default());
            let commands =
                renderer.update_buffers(&device, &queue, &mut encoder, &primitives, &screen);
            queue.submit(commands.into_iter().chain([encoder.finish()]));
        };
        let cut = crate::SurroundCut { amount: 1.0, reach: 8.0, shape: 0.0, tint_fade: 1.0 };
        let drawing = || {
            vec![
                crate::roll_paint_callback(
                    full,
                    full.size(),
                    vec![crate::roll::tests::centered_note()],
                    1,
                    full,
                    crate::RollAxes { pitch_dir: [1.0, 0.0], depth_dir: [0.0, 1.0] },
                    1.0,
                    cut,
                    FORMAT,
                    0,
                    1.0,
                ),
                crate::glow_paint_callback(
                    full,
                    vec![crate::GlowDot {
                        center: [128.0, 128.0],
                        radius: 12.0,
                        color: [200, 120, 60, 255],
                    }],
                    1.0,
                    FORMAT,
                ),
                crate::dot_surround_paint_callback(
                    full,
                    vec![crate::SurroundDot {
                        center: [128.0, 128.0],
                        radius: 4.0,
                        aura: [255, 0, 0, 255],
                    }],
                    cut,
                    FORMAT,
                ),
                crate::text_paint_callback(
                    full,
                    vec![crate::text::tests::glyph()],
                    crate::GlyphRuns::default(),
                    None,
                    crate::SheetUploads { font: Some(crate::text::tests::atlas()), marks: None },
                    crate::SlideAxis::default(),
                    FORMAT,
                    0,
                    1.0,
                ),
                crate::spectrogram::tests::cloud_paint_callback(),
            ]
        };
        let every = ["cloud", "dot surround", "glow", "roll", "spectrogram", "text"];

        let mut unprepared = egui_wgpu::Renderer::new(&device, FORMAT, Default::default());
        take_builds();
        frame(&mut unprepared, drawing());
        assert_eq!(take_builds(), every, "the control frame must reach every build");

        let mut renderer = egui_wgpu::Renderer::new(&device, FORMAT, Default::default());
        renderer.callback_resources.insert(instance.clone());
        let cache = Arc::new(LatticePipelineCache::default());
        let loading = WindowStartup::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while let Status::Preparing(_) = loading.status() {
            assert!(std::time::Instant::now() < deadline, "loading did not finish");
            frame(&mut renderer, vec![loading.callback(full, cache.clone(), FORMAT)]);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(loading.status(), Status::Ready { built: true });
        take_builds();
        frame(&mut renderer, drawing());
        assert_eq!(take_builds(), Vec::<&str>::new(), "the first drawing frame compiled these");

        // A reopened window: the cache answers in its first loading frame, and
        // its first drawing frame, with notes already on the roll, compiles
        // nothing either.
        let mut reopened = egui_wgpu::Renderer::new(&device, FORMAT, Default::default());
        reopened.callback_resources.insert(instance.clone());
        let loading = WindowStartup::default();
        frame(&mut reopened, vec![loading.callback(full, cache.clone(), FORMAT)]);
        assert_eq!(loading.status(), Status::Ready { built: false });
        frame(&mut reopened, drawing());
        assert_eq!(take_builds(), Vec::<&str>::new(), "the reopened window compiled these");
    }

    #[test]
    fn plugin_shutdown_joins_the_worker_and_prevents_restart() {
        let Some((instance, adapter)) = crate::test_gpu_adapter() else {
            return;
        };
        // The queue is unused now that the compiled resources upload nothing,
        // but it owns half the device handle and must outlive the worker.
        let (device, _queue) =
            pollster::block_on(adapter.request_device(&crate::device_descriptor(&adapter)))
                .unwrap();
        let cache = Arc::new(LatticePipelineCache::default());
        let (release, blocked) = std::sync::mpsc::channel();
        let worker_device = device.clone();
        let worker = std::thread::spawn(move || {
            blocked.recv().unwrap();
            super::Compiled::new(&worker_device, wgpu::TextureFormat::Bgra8Unorm)
        });
        cache.startup.lock().unwrap().job = Some(Job {
            instance: instance.clone(),
            device: device.clone(),
            format: wgpu::TextureFormat::Bgra8Unorm,
            progress: Arc::new(Mutex::new(Stage::Graphics)),
            worker: Some(worker),
            failed: false,
        });
        let (started, entering) = std::sync::mpsc::channel();
        let (finished, completion) = std::sync::mpsc::channel();
        let stopping = cache.clone();
        let shutdown = std::thread::spawn(move || {
            started.send(()).unwrap();
            stopping.shutdown_startup();
            finished.send(()).unwrap();
        });
        entering.recv().unwrap();
        assert!(
            completion.recv_timeout(std::time::Duration::from_millis(100)).is_err(),
            "plugin teardown returned while initialization was still blocked"
        );
        release.send(()).unwrap();
        completion.recv_timeout(std::time::Duration::from_secs(30)).unwrap();
        shutdown.join().unwrap();
        assert_eq!(
            cache.poll_startup(&instance, &device, wgpu::TextureFormat::Bgra8Unorm),
            Status::Failed,
            "a late window callback must not restart initialization during unload"
        );
    }
}

#[cfg(not(feature = "hot-reload"))]
pub(super) struct Job {
    instance: wgpu::Instance,
    device: wgpu::Device,
    format: wgpu::TextureFormat,
    progress: Arc<Mutex<Stage>>,
    worker: Option<std::thread::JoinHandle<Compiled>>,
    failed: bool,
}

#[cfg(not(feature = "hot-reload"))]
impl Drop for Job {
    fn drop(&mut self) {
        // The cache lives as long as the plugin instance. Join before its code
        // can be unloaded; dropping a window alone does not drop this job.
        // The worker owns device/queue handles, never the cache or an egui context.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(not(feature = "hot-reload"))]
impl LatticePipelineCache {
    pub(super) fn poll_startup(
        &self,
        instance: &wgpu::Instance,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> Status {
        let mut startup = self.startup.lock().expect("startup job poisoned");
        if startup.stopped {
            return Status::Failed;
        }
        let job = &mut startup.job;
        if let Some(active) = job.as_mut() {
            let same_device =
                active.instance == *instance && active.device == *device && active.format == format;
            if active.worker.as_ref().is_some_and(|worker| !worker.is_finished()) {
                return Status::Preparing(*active.progress.lock().expect("startup stage poisoned"));
            }
            if let Some(worker) = active.worker.take() {
                match worker.join() {
                    Ok(resources) => {
                        *self.compiled.lock().expect("lattice pipeline cache poisoned") =
                            Some((active.instance.clone(), active.device.clone(), resources));
                        *job = None;
                        if same_device {
                            return Status::Ready { built: true };
                        }
                    }
                    Err(_) => {
                        active.failed = true;
                        eprintln!("Harmonigraph graphics initialization worker failed");
                    }
                }
            }
            if job.as_ref().is_some_and(|active| active.failed) && same_device {
                return Status::Failed;
            }
            *job = None;
        }

        if self.compiled.lock().expect("lattice pipeline cache poisoned").as_ref().is_some_and(
            |(owner_instance, owner, resources)| {
                owner_instance == instance
                    && owner == device
                    && resources.lattice.target_format == format
            },
        ) {
            return Status::Ready { built: false };
        }

        let progress = Arc::new(Mutex::new(Stage::Graphics));
        let worker_progress = progress.clone();
        let worker_device = device.clone();
        let worker =
            std::thread::Builder::new().name("harmonigraph-graphics".into()).spawn(move || {
                Compiled::new_with_progress(&worker_device, format, |stage| {
                    *worker_progress.lock().expect("startup stage poisoned") = stage;
                })
            });
        match worker {
            Ok(worker) => {
                *job = Some(Job {
                    instance: instance.clone(),
                    device: device.clone(),
                    format,
                    progress,
                    worker: Some(worker),
                    failed: false,
                });
                Status::Preparing(Stage::Graphics)
            }
            Err(error) => {
                eprintln!(
                    "Could not start Harmonigraph graphics initialization: {error}; using synchronous initialization"
                );
                // No worker ran, so the loading callback can safely construct
                // resources through the existing synchronous cache path.
                // A panicked worker or shutdown must remain terminal instead.
                Status::Ready { built: false }
            }
        }
    }
}
