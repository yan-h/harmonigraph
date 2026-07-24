use std::{
    num::{NonZeroIsize, NonZeroU32},
    ptr::NonNull,
    sync::Arc,
};

use baseview::{PhySize, Window};
use egui::FullOutput;
use egui_wgpu::{
    RenderState, RendererOptions, ScreenDescriptor, WgpuError,
    wgpu::{
        Color, CommandEncoderDescriptor, Extent3d, RenderPassColorAttachment, RenderPassDescriptor,
        RenderPassTimestampWrites,
        Surface, SurfaceConfiguration, TextureDescriptor, TextureDimension, TextureUsages,
        TextureView, TextureViewDescriptor,
    },
};

pub use egui_wgpu::{WgpuConfiguration, WgpuSetup};

#[derive(Debug, Clone)]
pub struct GraphicsConfig {
    /// Controls whether to apply dithering to minimize banding artifacts.
    ///
    /// Dithering assumes an sRGB output and thus will apply noise to any input value that lies between
    /// two 8bit values after applying the sRGB OETF function, i.e. if it's not a whole 8bit value in "gamma space".
    /// This means that only inputs from texture interpolation and vertex colors should be affected in practice.
    ///
    /// Defaults to true.
    pub dithering: bool,

    /// Configures wgpu instance/device/adapter/surface creation and renderloop.
    pub wgpu_options: WgpuConfiguration,

    /// Additional options for the wgpu renderer.
    pub renderer_options: RendererOptions,
}

impl Default for GraphicsConfig {
    fn default() -> Self {
        Self {
            dithering: true,
            wgpu_options: Default::default(),
            renderer_options: Default::default(),
        }
    }
}

/// Two timestamps, 8 bytes each.
const EGUI_TIMER_BYTES: u64 = 16;

/// GPU time of egui's own render pass.
///
/// Both samples are BEGINNING-of-pass writes. Metal advertises and grants
/// `TIMESTAMP_QUERY_INSIDE_ENCODERS` and end-of-pass writes, then records ZERO
/// for both, silently — only a pass's opening sample comes back real. So the
/// bracket is egui's pass opening and the opening of a 1x1 no-op pass placed
/// after it.
///
/// The readback is a three-step cycle (record, map once the encoder is
/// submitted, read when the driver is done) and every poll is `Poll`, never
/// `Wait`: blocking for the number would stall the pipeline being measured.
/// The published value is a few frames old, which for "where is the frame
/// going" costs nothing.
struct EguiGpuTimer {
    set: egui_wgpu::wgpu::QuerySet,
    resolve: egui_wgpu::wgpu::Buffer,
    staging: egui_wgpu::wgpu::Buffer,
    /// 1x1 target for the trailing pass that carries the closing sample.
    tail: TextureView,
    period: f32,
    state: EguiTimerState,
    ready: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(PartialEq, Clone, Copy)]
enum EguiTimerState {
    Idle,
    Recorded,
    Mapping,
}

impl EguiGpuTimer {
    fn new(device: &egui_wgpu::wgpu::Device, queue: &egui_wgpu::wgpu::Queue) -> Option<Self> {
        use egui_wgpu::wgpu;
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        Some(EguiGpuTimer {
            set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("egui_gpu_timer"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("egui_gpu_timer_resolve"),
                size: EGUI_TIMER_BYTES,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            staging: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("egui_gpu_timer_staging"),
                size: EGUI_TIMER_BYTES,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            tail: device
                .create_texture(&TextureDescriptor {
                    label: Some("egui_gpu_timer_tail"),
                    size: Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&TextureViewDescriptor::default()),
            period: queue.get_timestamp_period(),
            state: EguiTimerState::Idle,
            ready: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    fn arming(&self) -> bool {
        self.state == EguiTimerState::Idle
    }

    fn poll(&mut self, device: &egui_wgpu::wgpu::Device) -> Option<f32> {
        use egui_wgpu::wgpu;
        use std::sync::atomic::Ordering;
        match self.state {
            EguiTimerState::Idle => None,
            EguiTimerState::Recorded => {
                let ready = self.ready.clone();
                self.staging.slice(..).map_async(wgpu::MapMode::Read, move |result| {
                    if result.is_ok() {
                        ready.store(true, Ordering::Release);
                    }
                });
                let _ = device.poll(wgpu::PollType::Poll);
                self.state = EguiTimerState::Mapping;
                None
            }
            EguiTimerState::Mapping => {
                let _ = device.poll(wgpu::PollType::Poll);
                if !self.ready.swap(false, Ordering::Acquire) {
                    return None;
                }
                let ms = {
                    // Read by hand rather than casting: this crate has no
                    // bytemuck, and it is two little-endian u64s.
                    let view = self.staging.slice(..).get_mapped_range();
                    let at = |i: usize| {
                        u64::from_le_bytes(view[i * 8..i * 8 + 8].try_into().unwrap_or_default())
                    };
                    // Saturating: both come off the same queue and should be
                    // ordered, but an out-of-order pair must not wrap.
                    let delta = at(1).saturating_sub(at(0)) as f64;
                    (delta * self.period as f64 / 1.0e6) as f32
                };
                self.staging.unmap();
                self.state = EguiTimerState::Idle;
                Some(ms)
            }
        }
    }

    /// Close the bracket with a 1x1 no-op pass and stage the result.
    fn close(&mut self, encoder: &mut egui_wgpu::wgpu::CommandEncoder) {
        use egui_wgpu::wgpu;
        encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("egui_gpu_timer_tail_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &self.tail,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: Some(RenderPassTimestampWrites {
                query_set: &self.set,
                beginning_of_pass_write_index: Some(1),
                end_of_pass_write_index: None,
            }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        encoder.resolve_query_set(&self.set, 0..2, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.staging, 0, EGUI_TIMER_BYTES);
        self.state = EguiTimerState::Recorded;
    }
}

pub struct Renderer {
    render_state: Arc<RenderState>,
    surface: Surface<'static>,
    config: GraphicsConfig,
    msaa_texture_view: Option<TextureView>,
    msaa_samples: u32,
    /// GPU time of egui's own render pass, and the queries behind it.
    gpu_timer: Option<EguiGpuTimer>,
    last_gpu_ms: f32,
    /// How long the last frame blocked acquiring the surface.
    last_acquire_ms: f32,
    /// Texture and buffer uploads, which is also where paint callbacks
    /// `prepare`; encoding egui's draw calls; and finish + submit + present.
    last_upload_ms: f32,
    /// Of that, the texture uploads alone — the rest is buffer uploads and,
    /// with them, the paint callbacks' `prepare`.
    last_texture_ms: f32,
    last_encode_ms: f32,
    last_submit_ms: f32,
    /// How long the last frame spent turning egui's shapes into triangles.
    ///
    /// Tessellation is neither the app's own per-frame work (it runs after the
    /// UI closure returns) nor GPU time, so without this it falls in a gap
    /// where a cost can hide from every other measurement.
    last_tess_ms: f32,
    width: u32,
    height: u32,
}

impl Renderer {
    pub fn new(window: &Window, config: GraphicsConfig) -> Result<Self, WgpuError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());

        let target = baseview_window_to_surface_target(window);
        let surface = unsafe { instance.create_surface_unsafe(target) }.unwrap();

        let msaa_samples = config.renderer_options.msaa_samples;

        let state = Arc::new(pollster::block_on(RenderState::create(
            &config.wgpu_options,
            &instance,
            Some(&surface),
            config.renderer_options,
        ))?);

        let gpu_timer = EguiGpuTimer::new(&state.device, &state.queue);

        Ok(Self {
            render_state: state,
            surface,
            config,
            msaa_texture_view: None,
            msaa_samples,
            gpu_timer,
            last_gpu_ms: 0.0,
            last_acquire_ms: 0.0,
            last_upload_ms: 0.0,
            last_texture_ms: 0.0,
            last_encode_ms: 0.0,
            last_submit_ms: 0.0,
            last_tess_ms: 0.0,
            width: 0,
            height: 0,
        })
    }

    /// Milliseconds the GPU spent on egui's own render pass, a few frames ago,
    /// or 0 where the device can't measure it. This is the 2D UI — dock,
    /// panels, text, the spectrogram quad, every roll ribbon — and is NOT
    /// covered by the lattice's own timer, which brackets only its passes.
    pub fn last_gpu_ms(&self) -> f32 {
        self.last_gpu_ms
    }

    /// Milliseconds the last frame spent blocked in `get_current_texture`.
    ///
    /// On a vsync-throttled surface this is where a frame that is ready too
    /// early waits for the display, so it is the difference between "we are
    /// slow" and "we are early" — and it appears in no cost measurement.
    pub fn last_acquire_ms(&self) -> f32 {
        self.last_acquire_ms
    }

    /// The renderer's remaining stages, in milliseconds: uploads (including
    /// paint callbacks' `prepare`), encoding egui's draw calls, and
    /// finish + submit + present.
    pub fn last_upload_ms(&self) -> f32 {
        self.last_upload_ms
    }

    /// Of the uploads, the TEXTURE half. `last_upload_ms` minus this is the
    /// buffer half, which is also where paint callbacks `prepare`.
    pub fn last_texture_ms(&self) -> f32 {
        self.last_texture_ms
    }

    pub fn last_encode_ms(&self) -> f32 {
        self.last_encode_ms
    }

    pub fn last_submit_ms(&self) -> f32 {
        self.last_submit_ms
    }

    /// Milliseconds the last frame spent in [`egui::Context::tessellate`].
    pub fn last_tess_ms(&self) -> f32 {
        self.last_tess_ms
    }

    pub fn max_texture_side(&self) -> usize {
        self.render_state
            .as_ref()
            .device
            .limits()
            .max_texture_dimension_2d as usize
    }

    fn configure_surface(&self, width: u32, height: u32) {
        let usage = TextureUsages::RENDER_ATTACHMENT;

        let mut surf_config = SurfaceConfiguration {
            usage,
            format: self.render_state.target_format,
            present_mode: self.config.wgpu_options.surface.present_mode,
            view_formats: vec![self.render_state.target_format],
            ..self
                .surface
                .get_default_config(&self.render_state.adapter, width, height)
                .expect("Unsupported surface")
        };

        if let Some(desired_maximum_frame_latency) = self
            .config
            .wgpu_options
            .surface
            .desired_maximum_frame_latency
        {
            surf_config.desired_maximum_frame_latency = desired_maximum_frame_latency;
        }

        self.surface
            .configure(&self.render_state.device, &surf_config);
    }

    fn resize_and_generate_msaa_view(&mut self, width: u32, height: u32) {
        let render_state = self.render_state.as_ref();

        self.width = width;
        self.height = height;

        self.configure_surface(width, height);

        let texture_format = render_state.target_format;

        if self.msaa_samples > 1 {
            self.msaa_texture_view = Some(
                render_state
                    .device
                    .create_texture(&TextureDescriptor {
                        label: Some("egui_msaa_texture"),
                        size: Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: self.msaa_samples.max(1),
                        dimension: TextureDimension::D2,
                        format: texture_format,
                        usage: TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[texture_format],
                    })
                    .create_view(&TextureViewDescriptor::default()),
            );
        }
    }

    /// Returns whether a frame was actually presented; `false` means the
    /// surface wasn't available (occluded window, outdated/lost surface)
    /// and the caller should retry rather than treat the frame as shown.
    pub fn render(
        &mut self,
        window: &baseview::Window<'_>,
        bg_color: egui::Rgba,
        physical_size: PhySize,
        pixels_per_point: f32,
        egui_ctx: &mut egui::Context,
        full_output: &mut FullOutput,
    ) -> bool {
        let PhySize {
            width: canvas_width,
            height: canvas_height,
        } = physical_size;

        // Advance last frame's readback before anything else: its encoder has
        // been submitted by now, so the map can be asked for and a landed
        // result published.
        if let Some(timer) = self.gpu_timer.as_mut() {
            if let Some(ms) = timer.poll(&self.render_state.device) {
                self.last_gpu_ms = ms;
            }
        }

        let shapes = std::mem::take(&mut full_output.shapes);

        let tess_start = std::time::Instant::now();
        let clipped_primitives = egui_ctx.tessellate(shapes, pixels_per_point);
        self.last_tess_ms = tess_start.elapsed().as_secs_f32() * 1000.0;
        let upload_start = std::time::Instant::now();

        let mut encoder =
            self.render_state
                .device
                .create_command_encoder(&CommandEncoderDescriptor {
                    label: Some("encoder"),
                });

        let screen_descriptor = ScreenDescriptor {
            size_in_pixels: [canvas_width, canvas_height],
            pixels_per_point,
        };

        let user_cmd_bufs = {
            let mut renderer = self.render_state.renderer.write();
            let tex_start = std::time::Instant::now();
            for (id, image_delta) in &full_output.textures_delta.set {
                // NOTE: `update_buffers` is also where egui-wgpu runs paint
            // callbacks' `prepare`, so the lattice's buffer writes — and the
            // GPU timer's `device.poll` — are inside this reading too. A
            // measurement that pays for itself has to be visible somewhere.
            renderer.update_texture(
                    &self.render_state.device,
                    &self.render_state.queue,
                    *id,
                    image_delta,
                );
            }

            self.last_texture_ms = tex_start.elapsed().as_secs_f32() * 1000.0;

            renderer.update_buffers(
                &self.render_state.device,
                &self.render_state.queue,
                &mut encoder,
                &clipped_primitives,
                &screen_descriptor,
            )
        };

        if self.width != canvas_width
            || self.height != canvas_height
            || self.msaa_texture_view.is_none()
        {
            self.resize_and_generate_msaa_view(canvas_width, canvas_height);
        }

        let mut recreate_surface = false;
        self.last_upload_ms = upload_start.elapsed().as_secs_f32() * 1000.0;

        // Timed because this is where a vsync-throttled frame WAITS. With a
        // Fifo surface, acquiring blocks until the display frees a slot, and
        // that wait is neither CPU work nor GPU work — it shows up in no other
        // reading, so a frame can be idle here while every cost row looks
        // cheap.
        let acquire_start = std::time::Instant::now();
        let acquired = self.surface.get_current_texture();
        self.last_acquire_ms = acquire_start.elapsed().as_secs_f32() * 1000.0;
        let output_frame = match acquired {
            wgpu::CurrentSurfaceTexture::Success(texture) => Some(texture),
            wgpu::CurrentSurfaceTexture::Occluded | wgpu::CurrentSurfaceTexture::Timeout => {
                // Flush this frame's staged uploads before bailing. The
                // update_texture/update_buffers calls above already pushed
                // egui's per-frame vertex, index and texture data into the
                // queue's pending writes, and those staging buffers are only
                // reclaimed by a submit() (via wgpu-core's pre_submit). Returning
                // without submitting strands them: a backgrounded window whose
                // surface reports Occluded/Timeout on every timer tick then piles
                // up staging buffers at the frame rate — gigabytes within minutes
                // — freed only when it regains focus and the next presented
                // frame's submit drains the whole backlog at once. No drawable
                // was acquired, so this submits the uploads without presenting;
                // the window stays frozen (expected while hidden) but flat.
                self.render_state
                    .queue
                    .submit(user_cmd_bufs.into_iter().chain([encoder.finish()]));
                return false;
            }
            wgpu::CurrentSurfaceTexture::Suboptimal(_) | wgpu::CurrentSurfaceTexture::Outdated => {
                None
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                unreachable!("No error scope registered, so validation errors will panic")
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                recreate_surface = true;
                None
            }
        };

        let Some(output_frame) = output_frame else {
            if recreate_surface {
                let target = baseview_window_to_surface_target(window);
                let instance =
                    wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
                self.surface = unsafe { instance.create_surface_unsafe(target) }.unwrap();
            }

            self.configure_surface(self.width, self.height);
            // Same staged-upload reclamation as the Occluded/Timeout arm above:
            // flush so pending writes don't accumulate across frames while the
            // surface stays unavailable (Suboptimal/Outdated/Lost).
            self.render_state
                .queue
                .submit(user_cmd_bufs.into_iter().chain([encoder.finish()]));
            return false;
        };

        // Skipped while a readback is still in flight, so the query set is
        // never overwritten mid-cycle.
        let timing = self.gpu_timer.as_ref().is_some_and(EguiGpuTimer::arming);
        {
            let renderer = self.render_state.renderer.read();
            let frame_view = output_frame
                .texture
                .create_view(&TextureViewDescriptor::default());

            let (view, resolve_target) = if let Some(msaa_view) = &self.msaa_texture_view {
                (msaa_view, Some(&frame_view))
            } else {
                (&frame_view, None)
            };

            let encode_start = std::time::Instant::now();
            let render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("egui_render"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target,
                    ops: egui_wgpu::wgpu::Operations {
                        load: egui_wgpu::wgpu::LoadOp::Clear(Color {
                            r: bg_color[0] as f64,
                            g: bg_color[1] as f64,
                            b: bg_color[2] as f64,
                            a: bg_color[3] as f64,
                        }),
                        store: egui_wgpu::wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: timing.then(|| RenderPassTimestampWrites {
                    query_set: &self.gpu_timer.as_ref().expect("timing implies a timer").set,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: None,
                }),
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // Forgetting the pass' lifetime means that we are no longer compile-time protected from
            // runtime errors caused by accessing the parent encoder before the render pass is dropped.
            // Since we don't pass it on to the renderer, we should be perfectly safe against this mistake here!
            renderer.render(
                &mut render_pass.forget_lifetime(),
                &clipped_primitives,
                &screen_descriptor,
            );
            self.last_encode_ms = encode_start.elapsed().as_secs_f32() * 1000.0;
        }

        if timing {
            if let Some(timer) = self.gpu_timer.as_mut() {
                timer.close(&mut encoder);
            }
        }

        {
            let mut renderer = self.render_state.renderer.write();
            for id in &full_output.textures_delta.free {
                renderer.free_texture(id);
            }
        }

        let submit_start = std::time::Instant::now();
        let encoded = encoder.finish();

        self.render_state
            .queue
            .submit(user_cmd_bufs.into_iter().chain([encoded]));

        output_frame.present();
        self.last_submit_ms = submit_start.elapsed().as_secs_f32() * 1000.0;
        true
    }
}

/// WGPU uses raw_window_handle v6, but baseview uses raw_window_handle v5, so manually convert it.
fn baseview_window_to_surface_target(window: &baseview::Window<'_>) -> wgpu::SurfaceTargetUnsafe {
    use raw_window_handle::{HasRawDisplayHandle, HasRawWindowHandle};

    let raw_display_handle = window.raw_display_handle();
    let raw_window_handle = window.raw_window_handle();

    wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: match raw_display_handle {
            raw_window_handle::RawDisplayHandle::AppKit(_) => {
                Some(raw_window_handle_06::RawDisplayHandle::AppKit(
                    raw_window_handle_06::AppKitDisplayHandle::new(),
                ))
            }
            raw_window_handle::RawDisplayHandle::Xlib(handle) => {
                Some(raw_window_handle_06::RawDisplayHandle::Xlib(
                    raw_window_handle_06::XlibDisplayHandle::new(
                        NonNull::new(handle.display),
                        handle.screen,
                    ),
                ))
            }
            raw_window_handle::RawDisplayHandle::Xcb(handle) => {
                Some(raw_window_handle_06::RawDisplayHandle::Xcb(
                    raw_window_handle_06::XcbDisplayHandle::new(
                        NonNull::new(handle.connection),
                        handle.screen,
                    ),
                ))
            }
            raw_window_handle::RawDisplayHandle::Windows(_) => {
                Some(raw_window_handle_06::RawDisplayHandle::Windows(
                    raw_window_handle_06::WindowsDisplayHandle::new(),
                ))
            }
            _ => todo!(),
        },
        raw_window_handle: match raw_window_handle {
            raw_window_handle::RawWindowHandle::AppKit(handle) => {
                raw_window_handle_06::RawWindowHandle::AppKit(
                    raw_window_handle_06::AppKitWindowHandle::new(
                        NonNull::new(handle.ns_view).unwrap(),
                    ),
                )
            }
            raw_window_handle::RawWindowHandle::Xlib(handle) => {
                raw_window_handle_06::RawWindowHandle::Xlib(
                    raw_window_handle_06::XlibWindowHandle::new(handle.window),
                )
            }
            raw_window_handle::RawWindowHandle::Xcb(handle) => {
                raw_window_handle_06::RawWindowHandle::Xcb(
                    raw_window_handle_06::XcbWindowHandle::new(
                        NonZeroU32::new(handle.window).unwrap(),
                    ),
                )
            }
            raw_window_handle::RawWindowHandle::Win32(handle) => {
                let mut raw_handle = raw_window_handle_06::Win32WindowHandle::new(
                    NonZeroIsize::new(handle.hwnd as isize).unwrap(),
                );

                raw_handle.hinstance = NonZeroIsize::new(handle.hinstance as isize);

                raw_window_handle_06::RawWindowHandle::Win32(raw_handle)
            }
            _ => todo!(),
        },
    }
}
