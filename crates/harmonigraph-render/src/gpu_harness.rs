//! The headless-GPU round trip every pixel test in this crate needs: a
//! device with no display attached, and the encode/submit/map sequence that
//! turns a paint callback's output into bytes to assert on. Shared by
//! `roll`, `text`, and the lattice suite — duplicating this per suite is
//! how the sequence quietly drifts right in one of them and wrong in
//! another, rather than being one thing to get right.
//!
//! Compiled into the library rather than the test build, because
//! [`crate::SpectrogramHeadless`] hands the same round trip to a dependent
//! crate: a parity test in `harmonigraph-ui` measures the shipping draw path
//! only if it goes through this one.

use crate::wgpu;

/// `None` where the machine has no usable GPU — CI containers, mostly.
/// Every caller returns on it.
pub(crate) fn headless_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let Ok(adapter) =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
    else {
        eprintln!("no GPU adapter available; skipping");
        return None;
    };
    let pair = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        .expect("headless device");
    Some(pair)
}

/// Render into a fresh texture cleared to `clear`, handing the pass to
/// `draw`, and return the texture for readback.
pub(crate) fn render_to_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    size: [u32; 2],
    format: wgpu::TextureFormat,
    clear: wgpu::Color,
    draw: impl FnOnce(&mut wgpu::RenderPass<'static>),
) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("parity_target"),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("parity_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        draw(&mut pass);
    }
    queue.submit([encoder.finish()]);
    texture
}

pub(crate) fn readback(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    size: [u32; 2],
) -> Vec<u8> {
    let bytes_per_row = size[0] * 4; // 256-wide RGBA rows are aligned
    assert_eq!(bytes_per_row % 256, 0, "test sizes keep rows aligned");
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("parity_readback"),
        size: (bytes_per_row * size[1]) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
    );
    queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("map readback buffer"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
    slice.get_mapped_range().to_vec()
}

/// Read an `R16Float` texture without row padding in the returned bytes.
#[cfg(test)]
pub(crate) fn readback_r16(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    size: [u32; 2],
) -> Vec<u8> {
    let row_bytes = size[0] * 2;
    let padded = row_bytes.div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("r16_readback"),
        size: u64::from(padded * size[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(size[1]),
            },
        },
        wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
    );
    queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("map R16 readback"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
    let mapped = slice.get_mapped_range();
    (0..size[1])
        .flat_map(|y| {
            let begin = (y * padded) as usize;
            mapped[begin..begin + row_bytes as usize].to_vec()
        })
        .collect()
}
