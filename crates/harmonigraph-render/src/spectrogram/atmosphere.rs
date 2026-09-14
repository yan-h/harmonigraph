//! A small scalar image diffuses the heatmap before its single palette lookup.
//! Targets belong to one pane and are keyed only on their size; source pixels and uniforms are refreshed every
//! draw, including paused zooms and palette edits.

use super::{create_spectrogram_pipeline, GridBuffer, SpectrogramUniforms, SpectrogramVertex};
use crate::{create_vertex_buffer, wgpu};

pub(super) const SOURCE: &str = include_str!("../shaders/spectral_atmosphere.wgsl");
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[derive(Clone, Copy, Debug)]
pub struct SpectrogramAtmosphere {
    pub settings: harmonigraph_scene::SpectralAtmosphere,
    /// The whole spectrogram region, including history that has no data yet.
    pub region: egui::Rect,
    /// Axis used to preserve the reduced source image's pitch footprint.
    pub pitch_vertical: bool,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    origin: [f32; 2],
    size: [f32; 2],
    step: [f32; 2],
    diffusion: f32,
    ppp: f32,
    /// How much of the soft field stands behind the Partials strokes. Read
    /// only by the composite and backdrop entry points; the four filter passes
    /// share this buffer and ignore it.
    cloud: f32,
    /// A uniform struct is laid out to 16, so the three words are the
    /// shader's own padding written where bytemuck can see them.
    _pad: [f32; 3],
}

pub(super) struct Pipelines {
    pub source: wgpu::RenderPipeline,
    pub bake: wgpu::RenderPipeline,
    pub composite: wgpu::RenderPipeline,
    pub backdrop: wgpu::RenderPipeline,
    filter_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    filters: [wgpu::RenderPipeline; 4],
    sampler: wgpu::Sampler,
}

impl Pipelines {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        source_layout: &wgpu::BindGroupLayout,
    ) -> Self {
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
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let uniform = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let filter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("spectral_cloud_filter_layout"),
            entries: &[texture(0), sampler_entry(1), uniform(2)],
        });
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("spectral_cloud_composite_layout"),
            entries: &[texture(0), texture(1), sampler_entry(2), uniform(3)],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spectral_cloud_filter"),
            source: wgpu::ShaderSource::Wgsl(SOURCE.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spectral_cloud_filter_pipeline_layout"),
            bind_group_layouts: &[Some(&filter_layout)],
            ..Default::default()
        });
        let filters = ["fs_close_h", "fs_close_v", "fs_wide_h", "fs_wide_v"].map(|entry| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_fullscreen"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: FORMAT,
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
        });
        Self {
            source: create_spectrogram_pipeline(
                device,
                FORMAT,
                source_layout,
                None,
                "fs_density_source",
            ),
            bake: create_spectrogram_pipeline(
                device,
                FORMAT,
                source_layout,
                Some(&composite_layout),
                "fs_cloud_light",
            ),
            composite: create_spectrogram_pipeline(
                device,
                format,
                source_layout,
                Some(&composite_layout),
                if format.is_srgb() || format == FORMAT {
                    "fs_cloud_linear"
                } else {
                    "fs_cloud_gamma"
                },
            ),
            backdrop: create_spectrogram_pipeline(
                device,
                format,
                source_layout,
                Some(&composite_layout),
                if format.is_srgb() || format == FORMAT {
                    "fs_cloud_backdrop_linear"
                } else {
                    "fs_cloud_backdrop_gamma"
                },
            ),
            filter_layout,
            composite_layout,
            filters,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("spectral_cloud_sampler"),
                min_filter: wgpu::FilterMode::Linear,
                mag_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
        }
    }
}

pub(super) struct Targets {
    pub size: [u32; 2],
    pub source_view: wgpu::TextureView,
    pub coverage_vertices: wgpu::Buffer,
    views: [wgpu::TextureView; 3],
    source_uniform: wgpu::Buffer,
    pub source_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    filter_groups: [wgpu::BindGroup; 3],
    pub bake_group: wgpu::BindGroup,
    pub composite_group: wgpu::BindGroup,
}

impl Targets {
    pub fn new(
        device: &wgpu::Device,
        pipelines: &Pipelines,
        size: [u32; 2],
        source_layout: &wgpu::BindGroupLayout,
        grid: &GridBuffer,
        lut: &wgpu::TextureView,
    ) -> Self {
        let view = |label| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let source_view = view("spectral_cloud_source");
        let views = [
            view("spectral_cloud_scratch"),
            view("spectral_cloud_close"),
            view("spectral_cloud_wide"),
        ];
        let buffer = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let source_uniform = buffer(
            "spectral_cloud_source_uniform",
            std::mem::size_of::<SpectrogramUniforms>() as u64,
        );
        let uniform = buffer("spectral_cloud_uniform", std::mem::size_of::<Uniforms>() as u64);
        let filter_groups = [&source_view, &views[0], &views[1]].map(|view| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("spectral_cloud_filter_group"),
                layout: &pipelines.filter_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&pipelines.sampler),
                    },
                    wgpu::BindGroupEntry { binding: 2, resource: uniform.as_entire_binding() },
                ],
            })
        });
        let cloud_group = |front| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("spectral_cloud_composite_group"),
                layout: &pipelines.composite_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(front),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&views[2]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&pipelines.sampler),
                    },
                    wgpu::BindGroupEntry { binding: 3, resource: uniform.as_entire_binding() },
                ],
            })
        };
        let bake_group = cloud_group(&views[1]);
        let composite_group = cloud_group(&source_view);
        let source_group = source_group(device, source_layout, &source_uniform, grid, lut);
        Self {
            size,
            source_view,
            coverage_vertices: create_vertex_buffer::<SpectrogramVertex>(
                device,
                "spectral_cloud_coverage",
                6,
            ),
            views,
            source_uniform,
            source_group,
            uniform,
            filter_groups,
            bake_group,
            composite_group,
        }
    }

    pub fn rebind(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        grid: &GridBuffer,
        lut: &wgpu::TextureView,
    ) {
        self.source_group = source_group(device, layout, &self.source_uniform, grid, lut);
    }

    pub fn update(
        &self,
        queue: &wgpu::Queue,
        mut read: SpectrogramUniforms,
        rect: egui::Rect,
        ppp: f32,
        atmosphere: SpectrogramAtmosphere,
    ) {
        // The source mesh records only measured history. Its already-blurred
        // light can occupy the whole spectrogram region, without crossing the
        // analyzer divider or widening the exact heatmap's sample footprint.
        let region = atmosphere.region.intersect(rect);
        let region = if region.is_positive() {
            region
        } else {
            egui::Rect::from_min_max(rect.min, rect.min)
        };
        let corners =
            [region.left_top(), region.right_top(), region.right_bottom(), region.left_bottom()];
        let vertices = [0, 1, 2, 0, 2, 3].map(|i| SpectrogramVertex {
            pos: corners[i].into(),
            slab: 0.0,
            t: 0.0,
        });
        queue.write_buffer(&self.coverage_vertices, 0, bytemuck::cast_slice(&vertices));
        read.origin_points = rect.min.into();
        read.viewport_points = rect.size().into();
        let pitch_vertical = atmosphere.pitch_vertical;
        let axis = usize::from(pitch_vertical);
        // A clipped pane can cover only part of the full pitch axis. Keep
        // that axis's bucket footprint per reduced pixel, not one full-range
        // footprint per texel of the smaller visible rectangle.
        let visible_pixels = if pitch_vertical { rect.height() } else { rect.width() } * ppp;
        read.rows =
            (read.rows as f32 * self.size[axis] as f32 / visible_pixels).round().max(1.0) as u32;
        queue.write_buffer(&self.source_uniform, 0, bytemuck::bytes_of(&read));
        let settings = atmosphere.settings.sanitized();
        let radius = rect.width().min(rect.height()) * 0.008;
        let uniforms = Uniforms {
            origin: rect.min.into(),
            size: rect.size().into(),
            step: [radius / rect.width(), radius / rect.height()],
            diffusion: settings.diffusion,
            ppp,
            cloud: settings.cloud,
            _pad: [0.0; 3],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniforms));
    }

    pub fn blur(&self, encoder: &mut wgpu::CommandEncoder, pipelines: &Pipelines) {
        // Source -> scratch -> close; close -> scratch -> wide. Feeding the
        // already softened image to the wide kernel closes its sampling gaps.
        // Every pass reads a different texture from the attachment it writes.
        for (i, (input, output)) in [(0, 0), (1, 1), (2, 0), (1, 2)].into_iter().enumerate() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("spectral_cloud_blur"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.views[output],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipelines.filters[i]);
            pass.set_bind_group(0, &self.filter_groups[input], &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

fn source_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform: &wgpu::Buffer,
    grid: &GridBuffer,
    lut: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("spectral_cloud_source_group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: grid.buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(lut) },
            wgpu::BindGroupEntry { binding: 3, resource: grid.peaks.as_entire_binding() },
        ],
    })
}
