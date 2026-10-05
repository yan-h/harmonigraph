//! Lattice's colored-light adapter for the shared star renderer.
use super::*;
use crate::stars::{self, StarUniforms};
const SOURCE: &str = concat!(
    include_str!("shaders/common.wgsl"),
    "\n",
    include_str!("shaders/stars.wgsl"),
    "\n",
    include_str!("shaders/lattice_stars.wgsl")
);

uniform_group! {
    struct Settings {
        stars: StarUniforms,
        depth: f32,
        _pad0: f32,
        _pad1: f32,
        _pad2: f32,
    }
}
#[derive(Clone)]
pub(super) struct Pipelines {
    output_layout: wgpu::BindGroupLayout,
    source_layout: wgpu::BindGroupLayout,
    image_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bake: wgpu::RenderPipeline,
    image: wgpu::RenderPipeline,
    material: wgpu::RenderPipeline,
}
impl Pipelines {
    pub(super) fn new(device: &wgpu::Device, output_layout: &wgpu::BindGroupLayout) -> Self {
        let texture = |binding, sample_type, view_dimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type, view_dimension, multisampled: false },
            count: None,
        };
        let float = wgpu::TextureSampleType::Float { filterable: true };
        let source_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lattice_star_source"),
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
                texture(1, float, wgpu::TextureViewDimension::D2),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let image_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("star_images"),
            entries: &[
                texture(0, wgpu::TextureSampleType::Uint, wgpu::TextureViewDimension::D2),
                texture(1, float, wgpu::TextureViewDimension::D2),
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lattice_stars"),
            source: wgpu::ShaderSource::Wgsl(SOURCE.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lattice_stars"),
            bind_group_layouts: &[Some(&source_layout), Some(&image_layout)],
            ..Default::default()
        });
        let pipeline = |name, format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(name),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_stars"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(name),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        Self {
            bake: pipeline("fs_star_bake", stars::STAR_FORMAT),
            image: pipeline("fs_stars", stars::STAR_IMAGE_FORMAT),
            material: pipeline("fs_lattice_stars", LATTICE_COLOR_FORMAT),
            output_layout: output_layout.clone(),
            source_layout,
            image_layout,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
        }
    }
}

pub(super) struct Frame {
    pub settings: harmonigraph_scene::StarSettings,
    pub direction: f32,
    pub now: f64,
    pub amount: f32,
}

#[derive(PartialEq, Eq)]
struct Allocation {
    output: [u32; 2],
    atlas: [u32; 2],
    image: [u32; 2],
}
pub(super) struct Targets {
    output: wgpu::TextureView,
    pub(super) output_group: wgpu::BindGroup,
    allocation: Allocation,
    atlas: wgpu::TextureView,
    image: wgpu::TextureView,
    uniform: wgpu::Buffer,
    source: wgpu::BindGroup,
    bake: wgpu::BindGroup,
    image_group: wgpu::BindGroup,
    material: wgpu::BindGroup,
}
impl Targets {
    fn new(
        device: &wgpu::Device,
        pipelines: &Pipelines,
        source: &wgpu::TextureView,
        allocation: Allocation,
    ) -> Self {
        let output =
            stars::image(device, "lattice_star_light", allocation.output, LATTICE_COLOR_FORMAT);
        let output_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lattice_star_light"),
            layout: &pipelines.output_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&output),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&pipelines.sampler),
                },
            ],
        });
        let atlas = stars::image(device, "star_atlas", allocation.atlas, stars::STAR_FORMAT);
        let image = stars::image(device, "star_image", allocation.image, stars::STAR_IMAGE_FORMAT);
        // A pass binds a stand-in for the target it renders into.
        let scratch = stars::image(device, "star_scratch", [1; 2], stars::STAR_IMAGE_FORMAT);
        let atlas_scratch = stars::image(device, "star_atlas_scratch", [1; 2], stars::STAR_FORMAT);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lattice_star_settings"),
            size: std::mem::size_of::<Settings>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let source = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lattice_star_source"),
            layout: &pipelines.source_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&pipelines.sampler),
                },
            ],
        });
        let group = |atlas: &wgpu::TextureView, tone: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("star_images"),
                layout: &pipelines.image_layout,
                entries: &[atlas, tone]
                    .into_iter()
                    .enumerate()
                    .map(|(binding, view)| wgpu::BindGroupEntry {
                        binding: binding as u32,
                        resource: wgpu::BindingResource::TextureView(view),
                    })
                    .collect::<Vec<_>>(),
            })
        };
        Self {
            bake: group(&atlas_scratch, &scratch),
            image_group: group(&atlas, &scratch),
            material: group(&atlas, &image),
            output,
            output_group,
            allocation,
            atlas,
            image,
            uniform,
            source,
        }
    }
    pub(super) fn prepare(
        held: &mut Option<Self>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipelines: &Pipelines,
        source: &wgpu::TextureView,
        size: [u32; 2],
        frame: Frame,
    ) {
        let Frame { settings: stars, direction, now, amount } = frame;
        let layout = stars::star_layout(stars, size[0] as f32 / size[1] as f32);
        let allocation = Allocation {
            output: size,
            atlas: stars::star_atlas_size(layout.size(), held.as_ref().map(|t| t.allocation.atlas)),
            image: stars::star_image_size(size, stars),
        };
        if held.as_ref().is_none_or(|t| t.allocation != allocation) {
            *held = Some(Self::new(device, pipelines, source, allocation));
        }
        let held = held.as_ref().unwrap();
        let settings = Settings {
            stars: StarUniforms::new(stars, direction, now, size, &layout, held.allocation.image),
            depth: amount,
            _pad0: 0.0,
            _pad1: 0.0,
            _pad2: 0.0,
        };
        queue.write_buffer(&held.uniform, 0, bytemuck::bytes_of(&settings));
    }
    pub(super) fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &Pipelines,
        has_light: bool,
    ) {
        if !has_light {
            // No retained color: clear previous stars without rebaking an empty field.
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("silent_lattice_stars"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.output,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            return;
        }
        for (view, pipeline, group) in [
            (&self.atlas, &pipelines.bake, &self.bake),
            (&self.image, &pipelines.image, &self.image_group),
            (&self.output, &pipelines.material, &self.material),
        ] {
            stars::Pass { view, pipeline, groups: &[&self.source, group], scissor: None }
                .draw(encoder);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shader_and_uniform_validate() {
        let module =
            naga::front::wgsl::parse_str(SOURCE).map_err(|e| e.emit_to_string(SOURCE)).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        crate::uniforms::layout::check_binding::<Settings>(SOURCE, 0, 0);
    }
}
