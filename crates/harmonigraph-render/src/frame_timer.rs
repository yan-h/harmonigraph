//! The GPU frame timer the `#[ignore]`d timing probes share: the lattice's
//! (`lattice_tests/timing.rs`), the roll's (`roll_timing.rs`) and the
//! spectrogram's (`spectrogram/tests/timing.rs`). One copy, because the two
//! Metal quirks below each make a probe report a wrong number without failing,
//! and a fix to one copy of them leaves another copy wrong (#1460).
//!
//! A frame is bracketed by two timestamps in one command buffer. Slot 0 opens
//! it: by default an empty 1x1 pass encoded ahead of `prepare`
//! ([`Opening::AheadOfPrepare`]); the spectrogram's probe opens on its first
//! real source pass instead ([`Opening::InPrepare`]). Slot 1 closes it at the
//! END of the pass `paint` draws into, and that pass has to READ what
//! `prepare` wrote: on a tile-based GPU a later pass's work overlaps an earlier
//! pass's fragments, so a stamp on an independent pass lands before them and
//! leaves them out (#1113).
//!
//! The closing pass always ends in one draw covering no pixel, because Metal
//! leaves the end stamp of a pass with no draw unwritten — an empty roll's
//! paint draws nothing — and the pair then reads as unsupported.

use crate::*;

/// Frames timed before any sample is kept: pipeline compilation, first
/// allocations and the GPU's clock coming up.
pub(crate) const WARM_UP: usize = 10;

/// Where slot 0, the frame's opening stamp, is written.
pub(crate) enum Opening<'a> {
    /// An empty 1x1 pass encoded ahead of `prepare`.
    AheadOfPrepare,
    /// A pass `prepare` encodes may write it; the closure, called after
    /// `prepare`, says whether one did. If none did, the closing pass's own
    /// beginning opens the frame.
    InPrepare(&'a dyn Fn() -> bool),
}

/// One frame's readings.
pub(crate) struct Sample {
    /// GPU time from the opening stamp to the end of the closing pass.
    pub(crate) gpu_ms: f64,
    /// Wall clock from submission through the readback's map: host
    /// submission and waiting included.
    pub(crate) completion_ms: f64,
    /// CPU time of the `prepare` closure.
    pub(crate) prepare_cpu_ms: f64,
}

/// The pane `paint` draws into, held across frames as a swapchain's is.
pub(crate) struct Pane {
    view: wgpu::TextureView,
    pub(crate) size: [u32; 2],
    pub(crate) ppp: f32,
    /// The pane in points, as egui hands it to a callback.
    pub(crate) rect: egui::Rect,
    pub(crate) screen: ScreenDescriptor,
}

impl Pane {
    pub(crate) fn paint_info(&self) -> egui::PaintCallbackInfo {
        egui::PaintCallbackInfo {
            viewport: self.rect,
            clip_rect: self.rect,
            pixels_per_point: self.ppp,
            screen_size_px: self.size,
        }
    }
}

pub(crate) struct FrameTimer {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub(crate) format: wgpu::TextureFormat,
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    staging: wgpu::Buffer,
    opening: wgpu::TextureView,
    no_pixel: wgpu::RenderPipeline,
    period: f64,
}

const STAMPS: u32 = 2;

const NO_PIXEL_SRC: &str = "
@vertex fn vs() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
@fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(0.0); }
";

impl FrameTimer {
    /// `None`, having said why, when there is no adapter or it carries no
    /// timestamps.
    pub(crate) fn new(format: wgpu::TextureFormat) -> Option<Self> {
        crate::shader_assets::initialize();
        let instance = wgpu::Instance::default();
        let Ok(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        else {
            eprintln!("no GPU adapter; nothing timed");
            return None;
        };
        eprintln!("adapter: {:?}", adapter.get_info());
        let features = wgpu::Features::TIMESTAMP_QUERY;
        if !adapter.features().contains(features) {
            eprintln!("the adapter carries no timestamps; nothing timed");
            return None;
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: crate::device_limits(&adapter),
            required_features: features,
            ..Default::default()
        }))
        .expect("a device with timestamps");
        let set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("frame_timer"),
            ty: wgpu::QueryType::Timestamp,
            count: STAMPS,
        });
        let buffer = |label, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: u64::from(STAMPS) * 8,
                usage,
                mapped_at_creation: false,
            })
        };
        let resolve = buffer(
            "frame_timer_resolve",
            wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        );
        let staging = buffer(
            "frame_timer_staging",
            wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("frame_timer_no_pixel"),
            source: wgpu::ShaderSource::Wgsl(NO_PIXEL_SRC.into()),
        });
        let no_pixel = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("frame_timer_no_pixel"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let period = f64::from(queue.get_timestamp_period());
        let opening = target(&device, format, "frame_timer_opening", [1, 1]);
        Some(Self { device, queue, format, set, resolve, staging, opening, no_pixel, period })
    }

    /// A pane of `size` device pixels at `ppp` pixels per point.
    pub(crate) fn pane(&self, size: [u32; 2], ppp: f32) -> Pane {
        let rect = egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(size[0] as f32 / ppp, size[1] as f32 / ppp),
        );
        Pane {
            view: target(&self.device, self.format, "frame_timer_pane", size),
            size,
            ppp,
            rect,
            screen: ScreenDescriptor { size_in_pixels: size, pixels_per_point: ppp },
        }
    }

    /// The query set whose slot 0 an [`Opening::InPrepare`] pass writes.
    pub(crate) fn query_set(&self) -> &wgpu::QuerySet {
        &self.set
    }

    /// Encode, submit and read back one frame: the opening stamp, `prepare`
    /// (whatever the probe encodes ahead of the pane, returning the command
    /// buffers that go ahead of the encoder's), then the closing pass on
    /// `pane` with `paint` drawing into it.
    pub(crate) fn frame(
        &self,
        pane: &Pane,
        resources: &mut CallbackResources,
        opening: Opening,
        prepare: impl FnOnce(
            &mut wgpu::CommandEncoder,
            &mut CallbackResources,
        ) -> Vec<wgpu::CommandBuffer>,
        paint: impl FnOnce(&mut wgpu::RenderPass<'static>, &CallbackResources),
    ) -> Sample {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        if let Opening::AheadOfPrepare = opening {
            drop(self.stamped_pass(&mut encoder, &self.opening, Some(0), None));
        }
        let prepare_start = std::time::Instant::now();
        let buffers = prepare(&mut encoder, resources);
        let prepare_cpu_ms = prepare_start.elapsed().as_secs_f64() * 1000.0;
        let opened = match opening {
            Opening::AheadOfPrepare => true,
            Opening::InPrepare(opened) => opened(),
        };
        {
            let mut pass =
                self.stamped_pass(&mut encoder, &pane.view, (!opened).then_some(0), Some(1));
            paint(&mut pass, resources);
            pass.set_pipeline(&self.no_pixel);
            pass.draw(0..3, 0..1);
        }
        encoder.resolve_query_set(&self.set, 0..STAMPS, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.staging, 0, u64::from(STAMPS) * 8);
        let completion_start = std::time::Instant::now();
        self.queue.submit(buffers.into_iter().chain([encoder.finish()]));
        let slice = self.staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        let completion_ms = completion_start.elapsed().as_secs_f64() * 1000.0;
        let ticks: Vec<u64> = bytemuck::cast_slice::<u8, u64>(&slice.get_mapped_range()).to_vec();
        self.staging.unmap();
        assert!(ticks[0] > 0 && ticks[1] >= ticks[0], "unsupported timestamp pair: {ticks:?}");
        Sample {
            gpu_ms: (ticks[1] - ticks[0]) as f64 * self.period / 1.0e6,
            completion_ms,
            prepare_cpu_ms,
        }
    }

    /// [`WARM_UP`] frames and then `frames` more, keeping the latter's
    /// samples. `frame` gets the frame's index from 0, warm-up included.
    pub(crate) fn time_frames(
        &self,
        frames: usize,
        mut frame: impl FnMut(usize) -> Sample,
    ) -> Vec<Sample> {
        (0..frames + WARM_UP).map(&mut frame).skip(WARM_UP).collect()
    }

    fn stamped_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        begin: Option<u32>,
        end: Option<u32>,
    ) -> wgpu::RenderPass<'static> {
        encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame_timer_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                    query_set: &self.set,
                    beginning_of_pass_write_index: begin,
                    end_of_pass_write_index: end,
                }),
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime()
    }
}

fn target(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    label: &str,
    size: [u32; 2],
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default())
}
