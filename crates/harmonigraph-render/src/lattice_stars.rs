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
        /// `Twinkle`, which the shared stars read through `star_twinkle`.
        star_twinkle: f32,
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
    pub layout: stars::StarLayout,
    /// How far the stars have travelled and their lives run, at
    /// [`stars::star_px_per_second`] (`PaneBuffers::material_travel`).
    pub motion: crate::travel::Motion,
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
    /// The atlas rows this frame's layout fills ([`stars::StarLayout::scissor`]):
    /// the allocation outlives the layout it was sized for.
    atlas_scissor: [u32; 4],
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
        let uniform = crate::gpu_memory::buffer(
            device,
            &wgpu::BufferDescriptor {
                label: Some("lattice_star_settings"),
                size: std::mem::size_of::<Settings>() as u64,
                usage: wgpu::BufferUsages::UNIFORM
                    | wgpu::BufferUsages::COPY_DST
                    | if cfg!(test) {
                        wgpu::BufferUsages::COPY_SRC
                    } else {
                        wgpu::BufferUsages::empty()
                    },
                mapped_at_creation: false,
            },
        );
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
            atlas_scissor: [0; 4],
            image,
            uniform,
            source,
        }
    }
    #[cfg(test)]
    pub(super) fn uniforms(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Settings {
        crate::gpu_harness::read_uniform(device, queue, &self.uniform)
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
        let Frame { settings: stars, layout, motion, amount } = frame;
        let allocation = Allocation {
            output: size,
            atlas: stars::star_atlas_size(layout.size(), held.as_ref().map(|t| t.allocation.atlas)),
            image: stars::star_image_size(size, stars),
        };
        if held.as_ref().is_none_or(|t| t.allocation != allocation) {
            *held = Some(Self::new(device, pipelines, source, allocation));
        }
        let held = held.as_mut().unwrap();
        held.atlas_scissor = layout.scissor();
        let settings = Settings {
            stars: StarUniforms::new(stars, motion, size, &layout, held.allocation.image),
            depth: amount,
            star_twinkle: stars.star_twinkle_dip,
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
        for (label, view, pipeline, group, scissor) in [
            (
                "lattice_star_atlas",
                &self.atlas,
                &pipelines.bake,
                &self.bake,
                Some(self.atlas_scissor),
            ),
            ("lattice_star_image", &self.image, &pipelines.image, &self.image_group, None),
            ("lattice_star_material", &self.output, &pipelines.material, &self.material, None),
        ] {
            stars::Pass { label, view, pipeline, groups: &[&self.source, group], scissor }
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

    /// Every lattice depth drifts along `Drift direction` at its own `Star
    /// speed`, and a depth at 1 at the prototype's pace: about a ninth of the
    /// pane's height a second. Equal speeds are no parallax at all.
    ///
    /// Read as SCREEN travel, `offset * cell`, which is what the eye sees —
    /// the offsets themselves are in each slice's own cells.
    #[test]
    fn stars_drift_with_parallax_at_the_prototypes_pace() {
        use stars::STAR_SLICES;
        // Held at the prototype's parallax, which is what the pace below is
        // stated against, whatever the fresh far speed is.
        let fresh = harmonigraph_scene::StarSettings {
            star_speed: [0.15, 0.2, 0.35, 0.6, 1.0],
            ..Default::default()
        };
        let slices = |settings: harmonigraph_scene::StarSettings, now| {
            let layout = stars::star_layout(settings, 16.0 / 9.0);
            let pace = stars::star_px_per_second();
            let rate = crate::travel::Motion::rate(1.0, 0.0, 1.0, settings, pace, layout.cells);
            let motion = crate::travel::Travel::default().advance(now, rate);
            stars::star_slices(settings, motion.stars, &layout, [1920, 1080])
        };
        let travelled = |settings, now| {
            slices(settings, now)
                .map(|slice| [slice.offset.0[0] * slice.cell, slice.offset.0[1] * slice.cell])
        };
        let near = stars::star_px_per_second() as f32 * 10.0;
        assert!((near / 540.0 / 10.0 - 0.114).abs() < 0.001);
        let moved = travelled(fresh, 10.0);
        assert!((moved[STAR_SLICES - 1][0] - near).abs() < 0.01, "{moved:?}");
        assert!((moved[0][0] - 0.15 * near).abs() < 0.01, "{moved:?}");
        assert!(moved.iter().all(|m| m[1].abs() < 1e-3));
        assert!(moved.windows(2).all(|w| w[0][0] < w[1][0]), "nearer is not faster: {moved:?}");
        let together = travelled(
            harmonigraph_scene::StarSettings { star_speed: [1.0; STAR_SLICES], ..fresh },
            10.0,
        );
        assert!(together.iter().all(|m| (m[0] - near).abs() < 0.01), "{together:?}");
        // A session left running for days still hands the shader an offset
        // inside one hash period rather than millions of pixels.
        for slice in slices(fresh, 3.0e5) {
            let period = stars::STAR_HASH_PERIOD as f32;
            assert!(slice.offset.0.iter().all(|&o| (0.0..=period).contains(&o)));
        }
    }
}
