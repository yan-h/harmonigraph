//! Watercolor and Mosaic sample the production note light through reusable pure geometry.
//! Tile lifetime belongs to the pane, separately from resize-dependent light.
use super::*;

const SOURCE: &str = concat!(
    include_str!("shaders/atmosphere_geometry.wgsl"),
    "\n",
    include_str!("shaders/lattice_material.wgsl"),
);
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[derive(Clone)]
pub(super) struct Pipelines {
    bake: wgpu::RenderPipeline,
    bake_layout: wgpu::BindGroupLayout,
    bake_mosaic: wgpu::RenderPipeline,
    mosaic: wgpu::RenderPipeline,
    material: wgpu::RenderPipeline,
    source_layout: wgpu::BindGroupLayout,
    tile_layout: wgpu::BindGroupLayout,
    source_sampler: wgpu::Sampler,
    tile_sampler: wgpu::Sampler,
}

impl Pipelines {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler = wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let source_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material_source_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture(1),
                sampler,
            ],
        });
        let tile_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lattice_material_tile_layout"),
            entries: &[texture(0), texture(1), sampler],
        });
        let bake_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material_geometry_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lattice_material"),
            source: wgpu::ShaderSource::Wgsl(SOURCE.into()),
        });
        let pipeline =
            |name, layouts: &[Option<&wgpu::BindGroupLayout>], formats: &[wgpu::TextureFormat]| {
                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(name),
                    bind_group_layouts: layouts,
                    ..Default::default()
                });
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(name),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_fullscreen"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(name),
                        compilation_options: Default::default(),
                        targets: &formats
                            .iter()
                            .map(|&format| {
                                Some(wgpu::ColorTargetState {
                                    format,
                                    blend: None,
                                    write_mask: wgpu::ColorWrites::ALL,
                                })
                            })
                            .collect::<Vec<_>>(),
                    }),
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleStrip,
                        ..Default::default()
                    },
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                })
            };
        Self {
            bake: pipeline("fs_tile", &[Some(&bake_layout)], &[FORMAT, FORMAT]),
            bake_mosaic: pipeline("fs_mosaic_tile", &[Some(&bake_layout)], &[FORMAT]),
            bake_layout,
            mosaic: pipeline(
                "fs_mosaic",
                &[Some(&source_layout), Some(&tile_layout)],
                &[LATTICE_COLOR_FORMAT],
            ),
            material: pipeline(
                "fs_material",
                &[Some(&source_layout), Some(&tile_layout)],
                &[LATTICE_COLOR_FORMAT],
            ),
            source_layout,
            tile_layout,
            source_sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            tile_sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
        }
    }
}

fn texture(
    device: &wgpu::Device,
    label: &str,
    size: [u32; 2],
    format: wgpu::TextureFormat,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default())
}
fn attachment(view: &wgpu::TextureView) -> Option<wgpu::RenderPassColorAttachment<'_>> {
    Some(wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            store: wgpu::StoreOp::Store,
        },
    })
}

pub(super) struct Tile {
    // Only baked geometry and quantized density belong in this key.
    // Refraction, layer mix, amount, drift, light and camera are live inputs.
    texels: u32,
    material: u32,
    geometry: [u32; 2],
    bind_group: wgpu::BindGroup,
}
impl Tile {
    #[cfg(test)]
    pub(super) fn geometry_binding(&self) -> wgpu::BindGroup {
        self.bind_group.clone()
    }
    fn new(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &Pipelines,
        texels: u32,
        material: u32,
        geometry: [u32; 2],
    ) -> Self {
        let a = texture(device, "lattice_material_tile_a", [texels; 2], FORMAT);
        let mosaic = material == harmonigraph_scene::LatticeMaterial::Mosaic as u32;
        let b = if mosaic {
            a.clone()
        } else {
            texture(device, "lattice_material_tile_b", [texels; 2], FORMAT)
        };
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lattice_material_tile"),
            layout: &pipelines.tile_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&a),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&b),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&pipelines.tile_sampler),
                },
            ],
        });
        use wgpu::util::DeviceExt;
        let values = if mosaic {
            [0.0, 0.0, f32::from_bits(geometry[0]), 0.0]
        } else {
            [f32::from_bits(geometry[0]), f32::from_bits(geometry[1]), 0.0, 0.0]
        };
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("material_geometry"),
            contents: bytemuck::cast_slice(&values),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bake_binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material_geometry"),
            layout: &pipelines.bake_layout,
            entries: &[wgpu::BindGroupEntry { binding: 3, resource: buffer.as_entire_binding() }],
        });
        let attachments =
            if mosaic { vec![attachment(&a)] } else { vec![attachment(&a), attachment(&b)] };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("lattice_material_geometry_bake"),
            color_attachments: &attachments,
            ..Default::default()
        });
        pass.set_pipeline(if mosaic { &pipelines.bake_mosaic } else { &pipelines.bake });
        pass.set_bind_group(0, &bake_binding, &[]);
        pass.draw(0..4, 0..1);
        Self { texels, material, geometry, bind_group }
    }
}

pub(super) struct Source {
    pub(super) view: wgpu::TextureView,
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}
impl Source {
    fn new(device: &wgpu::Device, pipelines: &Pipelines, size: [u32; 2]) -> Self {
        let view = texture(device, "lattice_material_light_source", size, LATTICE_COLOR_FORMAT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lattice_material_settings"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material_source"),
            layout: &pipelines.source_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&pipelines.source_sampler),
                },
            ],
        });
        Self { view, buffer, bind_group }
    }
    pub(super) fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &Pipelines,
        tile: &Tile,
        output: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("lattice_material_material"),
            color_attachments: &[attachment(output)],
            ..Default::default()
        });
        pass.set_pipeline(if tile.material == harmonigraph_scene::LatticeMaterial::Mosaic as u32 {
            &pipelines.mosaic
        } else {
            &pipelines.material
        });
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_bind_group(1, &tile.bind_group, &[]);
        pass.draw(0..4, 0..1);
    }
}

pub(super) fn prepare(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    pipelines: &Pipelines,
    pane: &mut PaneBuffers,
    settings: &MaterialParams,
) {
    let Some(offscreen) = pane.offscreen.as_mut() else { return };
    let Some(glow) = offscreen.glow.as_mut() else {
        pane.material_tile = None;
        return;
    };
    let displaced = settings.style == harmonigraph_scene::LatticeMaterial::Watercolor as u32
        || settings.style == harmonigraph_scene::LatticeMaterial::Mosaic as u32;
    if !displaced || settings.amount <= 0.0 {
        glow.material_source = None;
        pane.material_tile = None;
        return;
    }
    let size = offscreen.size.map(|n| n.div_ceil(2).max(1));
    // Match the spectrogram's pane-height calibration. Quantized density keeps
    // nearby resizes from rebaking; a 2048 cap bounds memory to 64 MiB.
    let watercolor = settings.style == harmonigraph_scene::LatticeMaterial::Watercolor as u32;
    let cell = size[1] as f32 * settings.scale / (10.0 * if watercolor { 5.25 } else { 6.0 / 2.2 });
    let geometry = if watercolor {
        [settings.fuzz.to_bits(), settings.lobe.to_bits()]
    } else {
        [settings.variety.to_bits(), 0]
    };
    let texels = ((40.0 * cell / 128.0).ceil() as u32 * 128).clamp(128, 2048);
    if pane.material_tile.as_ref().is_none_or(|tile| {
        tile.texels != texels || tile.material != settings.style || tile.geometry != geometry
    }) {
        pane.material_tile =
            Some(Tile::new(device, encoder, pipelines, texels, settings.style, geometry));
    }
    let source = glow.material_source.get_or_insert_with(|| Source::new(device, pipelines, size));
    let values = [
        size[0] as f32,
        size[1] as f32,
        cell,
        settings.amount,
        settings.drift.0[0],
        settings.drift.0[1],
        settings.refract,
        settings.layers,
        settings.randomness,
        0.0,
        0.0,
        0.0,
    ];
    queue.write_buffer(&source.buffer, 0, bytemuck::cast_slice(&values));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shader_validates() {
        let module =
            naga::front::wgsl::parse_str(SOURCE).map_err(|e| e.emit_to_string(SOURCE)).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    fn watercolor_brightness_stays_neutral_for_missing_globs() {
        use crate::gpu_harness::{headless_device, readback, render_to_texture};
        let Some((device, queue)) = headless_device() else { return };
        // Both points are inside the same lone fine glob, on opposite sides
        // of a cell boundary. No front bleed can mask the under-glob fallback.
        let source = format!(
            "{SOURCE}\n{}",
            r#"
@fragment
fn fs_brightness_boundary(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    var r = vec2<f32>(select(19.999, 20.001, p.x > 1.0), 8.0);
    if p.x > 2.0 {
        r = vec2<f32>(select(4.999, 5.001, p.x > 3.0), 58.2);
    }
    let scan = wash_scan(r, 2u, WASH_FINE_OCCUPANCY, 84);
    let wet = wash_wet(scan, r, 1.0, 2u, 84);
    let lone = all(scan.under == r) && scan.near <= -0.9;
    return vec4<f32>(0.5 + 0.5 * wet.brightness, scan.cover, select(0.0, 1.0, lone), 1.0);
}
"#
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("watercolor_brightness_boundary"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("watercolor_brightness_boundary"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_fullscreen"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_brightness_boundary"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let size = [4, 1];
        let target = render_to_texture(&device, &queue, size, format, wgpu::Color::BLACK, |pass| {
            pass.set_pipeline(&pipeline);
            pass.draw(0..4, 0..1);
        });
        let pixels = readback(&device, &queue, &target, size);
        for pixel in pixels[..8].chunks_exact(4) {
            assert_eq!(&pixel[1..3], &[255, 255], "fixture must reach a lone, covered glob");
        }
        // Uncovered tile texels still filter into their covered neighbors.
        for pixel in pixels[8..].chunks_exact(4) {
            assert_eq!(pixel, &[128, 0, 255, 255], "missing globs must have neutral brightness");
        }
        assert!(
            pixels[0].abs_diff(pixels[4]) <= 1,
            "a cell boundary inside one glob changed brightness: {pixels:?}"
        );
    }
}
