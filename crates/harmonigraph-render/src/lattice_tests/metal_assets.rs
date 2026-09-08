//! Manual metadata probe for the isolated backend hook in tools/metal-precompile.

use crate::wgpu;
use wgpu::util::DeviceExt;

/// The same storage buffer is bound at two different lengths. The result
/// depends on the actual binding range, arrayLength, and a generated bounds
/// check, rather than on the allocation size or a declared fixed array.
#[test]
#[ignore = "manual generated-Metal-asset probe; requires Metal and the copied backend"]
fn generated_metal_assets_preserve_storage_binding_lengths() {
    let (_, adapter) = crate::test_gpu_adapter().expect("Metal adapter required");
    assert_eq!(adapter.get_info().backend, wgpu::Backend::Metal);
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("metal_asset_binding_lengths"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
struct Input { prefix: vec4<u32>, values: array<u32> }
struct Output { values: vec4<u32> }
@group(0) @binding(0) var<storage, read> input: Input;
@group(0) @binding(1) var<storage, read_write> output: Output;
@compute @workgroup_size(1)
fn main() {
    let count = arrayLength(&input.values);
    output.values = vec4<u32>(count, input.values[0], input.values[count - 1u], input.values[99]);
}
"#
            .into(),
        ),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("metal_asset_binding_lengths"),
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let values = [100u32, 200, 300, 400, 11, 22, 33, 44, 55, 66, 77];
    let mut contents = vec![0u32; 64 + values.len()];
    contents[..values.len()].copy_from_slice(&values);
    contents[64..].copy_from_slice(&values);
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&contents),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    // The vec4 prefix makes this struct's minimum binding size 32 bytes.
    // Both ranges reach the shader, including a nonzero aligned buffer offset.
    for (offset, count) in [(0, 4u64), (0, 7), (256, 4), (256, 7)] {
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &input,
                        offset,
                        size: std::num::NonZeroU64::new(16 + count * 4),
                    }),
                },
                wgpu::BindGroupEntry { binding: 1, resource: output.as_entire_binding() },
            ],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 16);
        queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mapped = slice.get_mapped_range();
        let result: &[u32] = bytemuck::cast_slice(&mapped);
        // The pinned backend's Restrict policy clamps the out-of-range access
        // to the last element. This is a backend probe, not a portable WGSL
        // promise about which allowed value an out-of-bounds read produces.
        let last = count as u32 * 11;
        assert_eq!(result, &[count as u32, 11, last, last]);
        eprintln!("METAL_BINDING_LENGTH offset={offset} count={count} result={result:?}");
    }
}
