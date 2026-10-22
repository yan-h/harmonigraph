//! The lattice's GPU timestamp readback and the sentinels it publishes.

use super::*;

/// Wall-clock time the GPU spends on one pane's lattice passes, read back
/// with timestamp queries.
///
/// Deliberately a lagging measurement. The queries resolve into a buffer that
/// must be MAPPED to be read, mapping can only be requested once the encoder
/// is submitted (egui-wgpu owns the submit, one `prepare` later), and the map
/// completes whenever the driver gets to it. Blocking on any of that would
/// stall the very pipeline being measured, and the reading would then describe
/// a frame that was slow *because* it was timed. So it runs as a four-step
/// cycle and publishes a result a few frames old. For "is the GPU the
/// bottleneck", stale and honest beats fresh and self-inflicted.
pub(super) struct GpuTimer {
    pub(super) set: wgpu::QuerySet,
    /// `resolve_query_set` destination. Not mappable, hence the copy.
    pub(super) resolve: wgpu::Buffer,
    pub(super) staging: wgpu::Buffer,
    /// Nanoseconds per timestamp tick.
    pub(super) period: f32,
    pub(super) state: TimerState,
    /// Set once the timed frame's work has completed.
    pub(super) done: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 0 pending, 1 mapped, 2 failed; written by the map callback.
    pub(super) ready: std::sync::Arc<std::sync::atomic::AtomicU8>,
    /// The last opening stamp read, which an unwritten slot 0 would repeat.
    pub(super) last_opening: u64,
    /// 1x1 target carrying the opening timestamp.
    pub(super) head: wgpu::TextureView,
}

#[derive(PartialEq, Clone, Copy)]
pub(super) enum TimerState {
    /// Nothing in flight; the next frame may record.
    Idle,
    /// Queries sit in an encoder that has not been submitted yet.
    Recorded,
    /// Submitted, and waiting to hear that the frame has completed.
    Completing,
    /// Resolved, and the staging buffer has been asked to map.
    Mapping,
}

/// Two timestamps, 8 bytes each.
const TIMER_BYTES: u64 = 16;

/// Published in place of a measurement when the device can't do timestamp
/// queries at all.
///
/// A NaN bit pattern, as is [`GPU_TIME_PENDING`]. Zero would have been the
/// obvious sentinel and is the wrong choice: a real reading of 0.0 ms is
/// perfectly possible, and using it to mean "nothing yet" is what made a
/// landed-but-zero measurement indistinguishable from a stuck one.
pub const GPU_TIME_UNSUPPORTED: u32 = 0x7fc0_0001;

/// No new measurement has come back from the active timer.
pub const GPU_TIME_PENDING: u32 = 0x7fc0_0002;

/// No lattice pass is active; discard the previous interval's GPU readout.
pub const GPU_TIME_INACTIVE: u32 = 0x7fc0_0003;

impl GpuTimer {
    /// Build the query set and buffers, or `None` when the device can't.
    pub(super) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        Some(GpuTimer {
            set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("lattice_gpu_timer"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            }),
            resolve: crate::gpu_memory::buffer(
                device,
                &wgpu::BufferDescriptor {
                    label: Some("lattice_gpu_timer_resolve"),
                    size: TIMER_BYTES,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                },
            ),
            staging: Self::staging(device),
            period: queue.get_timestamp_period(),
            state: TimerState::Idle,
            done: Default::default(),
            ready: std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0)),
            last_opening: 0,
            head: crate::gpu_memory::texture(
                device,
                &wgpu::TextureDescriptor {
                    label: Some("lattice_gpu_timer_head"),
                    size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                },
            )
            .create_view(&Default::default()),
        })
    }

    fn staging(device: &wgpu::Device) -> wgpu::Buffer {
        crate::gpu_memory::buffer(
            device,
            &wgpu::BufferDescriptor {
                label: Some("lattice_gpu_timer_staging"),
                size: TIMER_BYTES,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        )
    }

    /// Advance the readback cycle, returning a measurement in milliseconds on
    /// the frame one finally lands.
    ///
    /// The stamps are resolved only once the timed frame has completed: a
    /// resolve submitted with the frame can read a slot before its stamp
    /// lands, which an M5 Pro under macOS 27 does nearly every frame, most
    /// often for the closing stamp. Such a slot keeps the last frame's value
    /// rather than reading 0, so a repeated opening stamp is refused too.
    pub(super) fn poll(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> Option<f32> {
        use std::sync::atomic::Ordering;
        match self.state {
            TimerState::Idle => None,
            TimerState::Recorded => {
                // The encoder holding those queries has been submitted by now,
                // so this hears about that frame's completion. That is true
                // because egui-wgpu submits once per frame and only ONE
                // callback per frame gets here — see
                // `LatticeCallback::drives_timer`.
                let done = self.done.clone();
                queue.on_submitted_work_done(move || done.store(true, Ordering::Release));
                let _ = device.poll(wgpu::PollType::Poll);
                self.state = TimerState::Completing;
                None
            }
            TimerState::Completing => {
                let _ = device.poll(wgpu::PollType::Poll);
                if !self.done.swap(false, Ordering::Acquire) {
                    return None;
                }
                let mut encoder = device.create_command_encoder(&Default::default());
                encoder.resolve_query_set(&self.set, 0..2, &self.resolve, 0);
                encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.staging, 0, TIMER_BYTES);
                queue.submit([encoder.finish()]);
                let ready = self.ready.clone();
                self.staging.slice(..).map_async(wgpu::MapMode::Read, move |result| {
                    ready.store(if result.is_ok() { 1 } else { 2 }, Ordering::Release);
                });
                // Poll, never Wait: a stall here would be the measurement
                // interfering with what it measures.
                let _ = device.poll(wgpu::PollType::Poll);
                self.state = TimerState::Mapping;
                None
            }
            TimerState::Mapping => {
                let _ = device.poll(wgpu::PollType::Poll);
                match self.ready.swap(0, Ordering::Acquire) {
                    0 => return None,
                    2 => {
                        // Like the host draw timer, replace a failed map's
                        // buffer: wgpu's API and core can disagree about its
                        // mapping state, so unmapping/retrying it is unsafe.
                        self.staging = Self::staging(device);
                        self.state = TimerState::Idle;
                        return None;
                    }
                    _ => {}
                }
                let ms = {
                    let view = self.staging.slice(..).get_mapped_range();
                    let ticks: &[u64] = bytemuck::cast_slice(&view);
                    let fresh = ticks[0] != std::mem::replace(&mut self.last_opening, ticks[0]);
                    Self::elapsed_ms(ticks[0], ticks[1], self.period).filter(|_| fresh)
                };
                self.staging.unmap();
                self.state = TimerState::Idle;
                ms
            }
        }
    }

    pub(super) fn elapsed_ms(begin: u64, end: u64, period: f32) -> Option<f32> {
        (begin != 0 && end != 0 && end >= begin)
            .then(|| ((end - begin) as f64 * f64::from(period) / 1.0e6) as f32)
    }

    /// Whether this frame should be timed — false while a readback is still
    /// in flight, so the query set is never overwritten mid-cycle.
    pub(super) fn arming(&self) -> bool {
        self.state == TimerState::Idle
    }

    /// Begin before lattice preparation. The closing stamp belongs to the
    /// last real preparation pass: scene without bloom, final blur with it.
    /// A separate tail pass has no dependency on the scene's fragment work
    /// and can begin before that work finishes on a tile-based GPU.
    pub(super) fn opening(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("lattice_gpu_timer_open_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.head,
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
                beginning_of_pass_write_index: Some(0),
                end_of_pass_write_index: None,
            }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }

    /// Close the preparation bracket; a later frame resolves it once this
    /// one completes.
    pub(super) fn close(&mut self) {
        self.state = TimerState::Recorded;
    }
}
