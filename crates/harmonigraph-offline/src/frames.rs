//! Rendering the UI with no window: a headless wgpu device, an egui
//! context driven by synthesized input, and an offscreen texture read
//! back to bytes.
//!
//! There is nothing exotic here, which is the point — the plugin's own
//! surface is the only thing this route does *not* need, and that surface
//! is exactly what blocks live capture (its usage flags don't allow a
//! copy, and neither its device nor its queue is reachable from the
//! editor). Everything else — the fonts, the panes, the lattice's paint
//! callback and all its GPU resources — works the same off a plain
//! `egui_wgpu::Renderer` as it does in the DAW.

use harmonigraph_render::wgpu;

/// The output pixel format. Non-sRGB deliberately: `egui-wgpu` picks its
/// gamma-correcting fragment shader based on `is_srgb()`, so an sRGB
/// target would shift every color away from what the plugin shows. RGBA
/// rather than the plugin's BGRA only so the bytes go straight to ffmpeg
/// without a swizzle; the two render identically.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Everything needed to render frames, set up once.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    egui: egui_wgpu::Renderer,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
    size: [u32; 2],
    /// Padded row stride of `readback`; wgpu requires 256-byte alignment,
    /// which an arbitrary width does not give us.
    bytes_per_row: u32,
}

/// Round `bytes` up to wgpu's copy alignment.
fn aligned(bytes: u32) -> u32 {
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    bytes.div_ceil(align) * align
}

impl Renderer {
    /// `size` is in physical pixels. Returns `None` if the machine has no
    /// usable GPU adapter (CI containers, mostly) — callers decide
    /// whether that is fatal.
    pub fn new(size: [u32; 2]) -> Option<Renderer> {
        let instance = wgpu::Instance::default();
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .ok()?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;

        // `predictable_texture_filtering` makes glyph sampling identical
        // across GPUs. It costs a little sharpness, but a render that
        // differs by machine would make the determinism test a lie and
        // make a re-render of one shot not match the others.
        let egui = egui_wgpu::Renderer::new(
            &device,
            FORMAT,
            egui_wgpu::RendererOptions {
                predictable_texture_filtering: true,
                ..Default::default()
            },
        );

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offline frame"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());

        let bytes_per_row = aligned(size[0] * 4);
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("offline readback"),
            size: u64::from(bytes_per_row) * u64::from(size[1]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Some(Renderer { device, queue, egui, target, view, readback, size, bytes_per_row })
    }

    /// Paint one frame's tessellated shapes and read the result back as
    /// tightly packed RGBA8 (row padding removed).
    pub fn render(
        &mut self,
        primitives: &[egui::ClippedPrimitive],
        textures: &egui::TexturesDelta,
        pixels_per_point: f32,
        clear: egui::Color32,
    ) -> Vec<u8> {
        for (id, delta) in &textures.set {
            self.egui.update_texture(&self.device, &self.queue, *id, delta);
        }

        let descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: self.size,
            pixels_per_point,
        };
        let mut encoder = self.device.create_command_encoder(&Default::default());
        // This is also where the lattice's paint callback runs its scene
        // and bloom passes — `update_buffers` dispatches `prepare` on our
        // encoder, and the callback creates its own GPU resources on
        // first use, so nothing here has to know the lattice exists.
        // Any command buffers a callback produced on its own encoder must
        // be submitted BEFORE the pass that reads their results.
        let callback_commands = self.egui.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            primitives,
            &descriptor,
        );

        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("offline frame"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(clear_value(clear)),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.egui.render(&mut pass, primitives, &descriptor);
        }

        encoder.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.bytes_per_row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: self.size[0],
                height: self.size[1],
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(callback_commands.into_iter().chain([encoder.finish()]));

        let slice = self.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map readback buffer"));
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        let row_bytes = (self.size[0] * 4) as usize;
        let frame = {
            let mapped = slice.get_mapped_range();
            let mut frame = Vec::with_capacity(row_bytes * self.size[1] as usize);
            for row in 0..self.size[1] as usize {
                let start = row * self.bytes_per_row as usize;
                frame.extend_from_slice(&mapped[start..start + row_bytes]);
            }
            frame
        };
        self.readback.unmap();

        for id in &textures.free {
            self.egui.free_texture(id);
        }
        frame
    }
}

/// The clear color, as wgpu wants it for [`FORMAT`].
///
/// Straight `byte / 255`, and that is the whole subtlety: because the
/// target is UNORM rather than sRGB, wgpu stores a clear value verbatim,
/// and egui — which picks its gamma-correcting fragment shader for
/// exactly the same reason — writes painted shapes as their gamma-space
/// bytes too. So both paths agree here. Converting to linear "because the
/// color is sRGB" would make the background come out several times too
/// dark, and only where the clear shows through.
fn clear_value(color: egui::Color32) -> wgpu::Color {
    let channel = |byte: u8| f64::from(byte) / 255.0;
    wgpu::Color {
        r: channel(color.r()),
        g: channel(color.g()),
        b: channel(color.b()),
        a: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_stride_is_padded_to_wgpus_alignment() {
        // 1920 and 3840 are already aligned; 1000 is not, and is exactly
        // the sort of width a hand-set --size would produce.
        assert_eq!(aligned(1920 * 4), 1920 * 4);
        assert_eq!(aligned(3840 * 4), 3840 * 4);
        assert_eq!(aligned(1000 * 4), 4096);
        assert_eq!(aligned(1) % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT, 0);
    }

    /// Pins the no-conversion decision: a UNORM target stores the clear
    /// verbatim, so the byte that goes in is the byte that comes out —
    /// matching what egui paints for the same color.
    #[test]
    fn the_clear_color_is_the_gamma_space_byte_not_a_linear_one() {
        let value = clear_value(egui::Color32::from_rgb(128, 0, 255));
        assert!((value.r - 128.0 / 255.0).abs() < 1e-9);
        assert_eq!(value.g, 0.0);
        assert_eq!(value.b, 1.0);
        assert_eq!(value.a, 1.0);
    }
}
