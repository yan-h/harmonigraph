//! One initialization worker per retained editor cache. Only pipeline construction
//! moves off the window thread; each window still owns fresh mutable resources.

#[cfg(not(feature = "hot-reload"))]
use std::sync::atomic::{AtomicBool, Ordering};
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
        if cfg!(feature = "hot-reload") {
            Self(Arc::new(Mutex::new(Status::Ready { built: false })))
        } else {
            Self::preparing()
        }
    }
}

impl WindowStartup {
    /// Progress at its first stage whatever this build's features, so a test
    /// reaches the loading screen even where `hot-reload` is unified in and
    /// [`Default`] starts at `Ready`.
    pub fn preparing() -> Self {
        Self(Arc::new(Mutex::new(Status::Preparing(Stage::Graphics))))
    }

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
        Self { lattice, panes: PanePipelines::new(device, format, &progress) }
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
    /// Each pane reports Interface again before it builds: no new label, but a
    /// boundary the worker can stop at, since the cloud alone measured 687 ms
    /// cold (#1743).
    pub(super) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        progress: impl Fn(Stage),
    ) -> Self {
        progress(Stage::Interface);
        let roll = crate::roll::RollPipelines::new(device, format);
        progress(Stage::Interface);
        let glow = crate::glow::GlowPipelines::new(device, format);
        progress(Stage::Interface);
        let dots = crate::dot_surround::Pipelines::new(device, format);
        progress(Stage::Interface);
        let text = crate::text::TextPipelines::new(device, format);
        progress(Stage::Interface);
        let spectrogram = crate::spectrogram::SpectrogramPipelines::new(device, format);
        progress(Stage::Interface);
        Self { roll, glow, dots, text, spectrogram: spectrogram.with_cloud(device) }
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
    /// Start compiling for `device` now, before any window asks: the plugin
    /// calls this at activation (#1746), so the first open finds the job done
    /// or under way on the device it reuses, rather than starting it cold.
    /// The window's loading callback then polls this same job. Nothing more
    /// happens when one is already running, or done, for this device.
    ///
    /// `egui` is the options the window will build its egui renderer with.
    /// The job builds one first, only to warm Metal's cache: each window builds
    /// its own on the host's main thread (egui-wgpu's `RenderState::create`)
    /// before any loading frame, so a job spares that thread a cold compile
    /// only by getting there first, which a job a window started cannot do.
    pub fn start(
        &self,
        instance: &wgpu::Instance,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        egui: egui_wgpu::RendererOptions,
    ) {
        #[cfg(not(feature = "hot-reload"))]
        let _ = self.startup(instance, device, format, Some(egui));
        #[cfg(feature = "hot-reload")]
        let _ = (instance, device, format, egui);
    }

    /// The instance and device the startup job compiles for, or compiled for:
    /// for another crate's test of who made that device.
    #[cfg(feature = "test-support")]
    pub fn startup_device(&self) -> Option<(wgpu::Instance, wgpu::Device)> {
        #[cfg(not(feature = "hot-reload"))]
        {
            let startup = self.startup.lock().expect("startup job poisoned");
            if let Some(job) = &startup.job {
                return Some((job.instance.clone(), job.device.clone()));
            }
            let compiled = self.compiled.lock().expect("lattice pipeline cache poisoned");
            compiled.as_ref().map(|(instance, device, _)| (instance.clone(), device.clone()))
        }
        #[cfg(feature = "hot-reload")]
        None
    }

    /// Join initialization before unloading plugin code, even if a window still
    /// retains shared UI state. Closing just the editor must not call this.
    /// The worker stops at its next stage boundary (`Job`'s drop), so a plugin
    /// destroyed mid-compile waits out one stage rather than the whole build.
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
        // Through egui-wgpu's own renderer, as the editor's frames are, with
        // egui's font texture published as the shells publish it.
        let font = crate::text::tests::font_texture(&device, &queue, &crate::text::tests::atlas());
        let frame = |renderer: &mut egui_wgpu::Renderer, callbacks: Vec<egui::PaintCallback>| {
            renderer.callback_resources.insert(font.clone());
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
                    None,
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

    /// The job activation starts (#1746) is the one a window opening on the
    /// same device polls: every loading frame finds that worker still running,
    /// or its finished build, and never starts a second. A second activation
    /// starts nothing either.
    #[test]
    fn a_window_polls_the_job_activation_started() {
        const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
        let Some((instance, adapter)) = crate::test_gpu_adapter() else {
            return;
        };
        let (device, _queue) =
            pollster::block_on(adapter.request_device(&crate::device_descriptor(&adapter)))
                .unwrap();
        let cache = LatticePipelineCache::default();
        let worker = |cache: &LatticePipelineCache| {
            let startup = cache.startup.lock().unwrap();
            startup.job.as_ref().and_then(|job| job.worker.as_ref()).map(|w| w.thread().id())
        };
        cache.start(&instance, &device, FORMAT, Default::default());
        let started = worker(&cache).expect("activation started no job");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            match cache.poll_startup(&instance, &device, FORMAT) {
                Status::Preparing(_) => {
                    assert_eq!(worker(&cache), Some(started), "the window started its own job");
                    assert!(std::time::Instant::now() < deadline, "loading did not finish");
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Status::Ready { built: true } => break,
                other => panic!("the window did not find activation's job: {other:?}"),
            }
        }
        cache.start(&instance, &device, FORMAT, Default::default());
        assert_eq!(worker(&cache), None, "a second activation started another compile");
    }

    /// Plugin destruction stops the worker at its next stage rather than
    /// waiting out the build: shutdown sets the stop before it joins, and the
    /// build unwinds at the boundary after the one it was in, building none of
    /// the stages behind it.
    #[test]
    fn plugin_shutdown_stops_the_worker_at_its_next_stage() {
        let Some((instance, adapter)) = crate::test_gpu_adapter() else {
            return;
        };
        let (device, _queue) =
            pollster::block_on(adapter.request_device(&crate::device_descriptor(&adapter)))
                .unwrap();

        // A worker that only ever ends when told to stop, as `compile` does.
        let cache = LatticePipelineCache::default();
        let stop = Arc::new(AtomicBool::new(false));
        let waiting = stop.clone();
        let worker = std::thread::spawn(move || -> super::Compiled {
            while !waiting.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            std::panic::resume_unwind(Box::new(super::Stopped))
        });
        cache.startup.lock().unwrap().job = Some(Job {
            instance,
            device: device.clone(),
            format: wgpu::TextureFormat::Bgra8Unorm,
            progress: Arc::new(Mutex::new(Stage::Graphics)),
            stop,
            worker: Some(worker),
            failed: false,
        });
        let (finished, completion) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            cache.shutdown_startup();
            finished.send(()).unwrap();
        });
        completion
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("shutdown joined a worker it never told to stop");

        // The build itself, told to stop at the second Interface report: the
        // lattice's last step is the first, and the second comes before the
        // roll, the first pane. So it builds the roll, unwinds at the next
        // pane's boundary and reports nothing after, which only the panes' own
        // boundaries make it do.
        take_builds();
        let stop = AtomicBool::new(false);
        let seen = Mutex::new(Vec::new());
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::compile(&device, wgpu::TextureFormat::Bgra8Unorm, None, &stop, |stage| {
                let mut seen = seen.lock().unwrap();
                seen.push(stage);
                if seen.iter().filter(|&&s| s == Stage::Interface).count() == 2 {
                    stop.store(true, Ordering::Relaxed);
                }
            })
        }));
        let stopped = built.err().expect("the build ran on past its stop");
        assert!(stopped.is::<super::Stopped>(), "it unwound with something else");
        let interface = seen.into_inner().unwrap().into_iter().filter(|&s| s == Stage::Interface);
        assert_eq!(interface.count(), 2, "it reported a stage after the stop");
        assert_eq!(take_builds(), ["roll"], "it stopped at another boundary");
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
            stop: Arc::new(AtomicBool::new(false)),
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
    /// Read by the worker at each stage boundary ([`compile`]).
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<Compiled>>,
    failed: bool,
}

#[cfg(not(feature = "hot-reload"))]
impl Drop for Job {
    fn drop(&mut self) {
        // The cache lives as long as the plugin instance. Join before its code
        // can be unloaded; dropping a window alone does not drop this job.
        // The worker owns device/queue handles, never the cache or an egui context.
        // Stopping it first bounds the wait at one stage: since activation
        // starts the job (#1746), a plugin deleted or a project closed soon
        // after loading would otherwise block the host's main thread here for
        // the rest of a cold compile, about 2.4 s, with no editor ever opened.
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// What a stopped worker unwinds with. Never a failure anyone reads: only a
/// dropped `Job` stops its worker, and that drop discards the join's result.
#[cfg(not(feature = "hot-reload"))]
struct Stopped;

/// The worker's whole build, reporting each stage to `progress`. At every
/// stage boundary it checks `stop` and, once that is set, unwinds out of the
/// build with [`Stopped`]: through `resume_unwind`, which skips the panic
/// hook, so nothing is printed. Unwinding rather than returning keeps the
/// constructors' stage reports free of an early-return path apiece.
#[cfg(not(feature = "hot-reload"))]
fn compile(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    egui: Option<egui_wgpu::RendererOptions>,
    stop: &AtomicBool,
    progress: impl Fn(Stage),
) -> Compiled {
    let stage = |stage| {
        if stop.load(Ordering::Relaxed) {
            std::panic::resume_unwind(Box::new(Stopped));
        }
        progress(stage);
    };
    if let Some(options) = egui {
        stage(Stage::Graphics);
        drop(egui_wgpu::Renderer::new(device, format, options));
    }
    Compiled::new_with_progress(device, format, stage)
}

#[cfg(not(feature = "hot-reload"))]
impl LatticePipelineCache {
    /// The window's poll: its egui renderer is already built.
    pub(super) fn poll_startup(
        &self,
        instance: &wgpu::Instance,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> Status {
        self.startup(instance, device, format, None)
    }

    fn startup(
        &self,
        instance: &wgpu::Instance,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        egui: Option<egui_wgpu::RendererOptions>,
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
        let stop = Arc::new(AtomicBool::new(false));
        let (worker_progress, worker_stop) = (progress.clone(), stop.clone());
        let worker_device = device.clone();
        let worker =
            std::thread::Builder::new().name("harmonigraph-graphics".into()).spawn(move || {
                compile(&worker_device, format, egui, &worker_stop, |stage| {
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
                    stop,
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
