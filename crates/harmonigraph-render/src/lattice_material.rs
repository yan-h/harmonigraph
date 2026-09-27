//! Watercolor and Mosaic sample the production note light through reusable pure geometry.
//! Tile lifetime belongs to the pane, separately from resize-dependent light.
use super::*;

const SOURCE: &str = concat!(
    include_str!("shaders/atmosphere_geometry.wgsl"),
    "\n",
    include_str!("shaders/lattice_material.wgsl"),
);
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Exactly representable in the instance's spare f32; never a strip row or a
/// camera-relative world coordinate. Three lattice axes distinguish sheets.
pub(super) fn node_seed(fives: i32, threes: i32, sevens: i32) -> f32 {
    let mut n = (fives as u32).wrapping_mul(0x9e37_79b9)
        ^ (threes as u32).wrapping_mul(0x85eb_ca6b)
        ^ (sevens as u32).wrapping_mul(0x27d4_eb2d);
    n = (n ^ (n >> 16)).wrapping_mul(0x7feb_352d);
    ((n ^ (n >> 15)) & 0x00ff_ffff) as f32
}

#[derive(Clone)]
pub(super) struct Pipelines {
    bake: wgpu::RenderPipeline,
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
            bake: pipeline("fs_tile", &[], &[FORMAT, FORMAT]),
            bake_mosaic: pipeline("fs_mosaic_tile", &[], &[FORMAT]),
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
    /// Keyed on material and quantized texels per fixed 40-cell period.
    /// Watercolor fuzz .25/lobe .7 and Mosaic variety .5 are shader constants.
    /// Depth, roughness, drift, light and camera never change geometry;
    /// size affects density only.
    texels: u32,
    material: u32,
    bind_group: wgpu::BindGroup,
}
impl Tile {
    pub(super) fn is_watercolor(&self) -> bool {
        self.material == harmonigraph_scene::LatticeMaterial::Watercolor as u32
    }
    fn new(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &Pipelines,
        texels: u32,
        material: u32,
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
        let attachments =
            if mosaic { vec![attachment(&a)] } else { vec![attachment(&a), attachment(&b)] };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("lattice_material_geometry_bake"),
            color_attachments: &attachments,
            ..Default::default()
        });
        pass.set_pipeline(if mosaic { &pipelines.bake_mosaic } else { &pipelines.bake });
        pass.draw(0..4, 0..1);
        Self { texels, material, bind_group }
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
            size: 32,
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
    settings: &NebulaParams,
) {
    let Some(offscreen) = pane.offscreen.as_mut() else { return };
    let Some(glow) = offscreen.glow.as_mut() else {
        pane.material_tile = None;
        return;
    };
    let displaced = settings.material == harmonigraph_scene::LatticeMaterial::Watercolor as u32
        || settings.material == harmonigraph_scene::LatticeMaterial::Mosaic as u32;
    if !displaced || settings.depth <= 0.0 {
        glow.material_source = None;
        pane.material_tile = None;
        return;
    }
    let size = offscreen.size.map(|n| n.div_ceil(2).max(1));
    // 23px cells at the prototype's 560px pane and size 1x. Quantized density
    // keeps nearby resizes from rebaking; a 2048 cap bounds memory to 64 MiB.
    let cell = size[1] as f32 * (23.0 / 560.0) * settings.scale;
    let texels = ((40.0 * cell / 128.0).ceil() as u32 * 128).clamp(128, 2048);
    if pane
        .material_tile
        .as_ref()
        .is_none_or(|tile| tile.texels != texels || tile.material != settings.material)
    {
        pane.material_tile = Some(Tile::new(device, encoder, pipelines, texels, settings.material));
    }
    let source = glow.material_source.get_or_insert_with(|| Source::new(device, pipelines, size));
    let values = [
        size[0] as f32,
        size[1] as f32,
        cell,
        settings.depth,
        settings.drift.0[0],
        settings.drift.0[1],
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
}
