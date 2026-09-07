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
            let status = self.cache.poll_startup(&instance, device, queue, self.format);
            if matches!(status, Status::Ready { .. }) {
                resources.insert(self.cache.resources(&instance, device, queue, self.format));
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

    #[test]
    fn plugin_shutdown_joins_the_worker_and_prevents_restart() {
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let cache = Arc::new(LatticePipelineCache::default());
        let (release, blocked) = std::sync::mpsc::channel();
        let worker_device = device.clone();
        let worker_queue = queue.clone();
        let worker = std::thread::spawn(move || {
            blocked.recv().unwrap();
            super::super::LatticeResources::new(
                &worker_device,
                &worker_queue,
                wgpu::TextureFormat::Bgra8Unorm,
            )
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
            cache.poll_startup(&instance, &device, &queue, wgpu::TextureFormat::Bgra8Unorm),
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
    worker: Option<std::thread::JoinHandle<super::LatticeResources>>,
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
        queue: &wgpu::Queue,
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
                    Ok(mut resources) => {
                        resources.timer = None;
                        *self.template.lock().expect("lattice pipeline cache poisoned") =
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

        if self.template.lock().expect("lattice pipeline cache poisoned").as_ref().is_some_and(
            |(owner_instance, owner, resources)| {
                owner_instance == instance && owner == device && resources.target_format == format
            },
        ) {
            return Status::Ready { built: false };
        }

        let progress = Arc::new(Mutex::new(Stage::Graphics));
        let worker_progress = progress.clone();
        let worker_device = device.clone();
        let worker_queue = queue.clone();
        let worker =
            std::thread::Builder::new().name("harmonigraph-graphics".into()).spawn(move || {
                super::LatticeResources::new_with_progress(
                    &worker_device,
                    &worker_queue,
                    format,
                    |stage| {
                        *worker_progress.lock().expect("startup stage poisoned") = stage;
                    },
                )
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
                eprintln!("Could not start Harmonigraph graphics initialization: {error}");
                Status::Failed
            }
        }
    }
}
