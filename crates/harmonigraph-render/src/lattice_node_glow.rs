//! Full-resolution directional halo quads and hardware-blended overlap statistics.

use super::*;

const FORMATS: [wgpu::TextureFormat; 3] = [
    wgpu::TextureFormat::Rgba16Float,
    wgpu::TextureFormat::Rg16Float,
    wgpu::TextureFormat::Rgba16Float,
];

pub(super) fn statistics_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("glow_statistics_layout"),
        entries: &std::array::from_fn::<_, 3, _>(|i| wgpu::BindGroupLayoutEntry {
            binding: i as u32,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }),
    })
}

pub(super) fn statistics(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    size: [u32; 2],
) -> ([wgpu::TextureView; 3], wgpu::BindGroup) {
    let views = FORMATS.map(|format| {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("glow_statistics"),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default())
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("glow_statistics_bind_group"),
        layout,
        entries: &std::array::from_fn::<_, 3, _>(|i| wgpu::BindGroupEntry {
            binding: i as u32,
            resource: wgpu::BindingResource::TextureView(&views[i]),
        }),
    });
    (views, bind_group)
}

pub(super) fn create_glow_pipelines(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    target_format: wgpu::TextureFormat,
    uniforms: &wgpu::BindGroupLayout,
    strip: &wgpu::BindGroupLayout,
    statistics: &wgpu::BindGroupLayout,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let blend = |dst_factor| {
        let component = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor,
            operation: wgpu::BlendOperation::Add,
        };
        wgpu::BlendState { color: component, alpha: component }
    };
    let targets = std::array::from_fn::<_, 3, _>(|i| {
        Some(wgpu::ColorTargetState {
            format: FORMATS[i],
            blend: Some(blend(if i == 0 {
                wgpu::BlendFactor::One
            } else {
                wgpu::BlendFactor::OneMinusSrc
            })),
            write_mask: wgpu::ColorWrites::ALL,
        })
    });
    let make = |vertex,
                fragment,
                second_layout,
                buffers: &[wgpu::VertexBufferLayout<'_>],
                targets: &[Option<wgpu::ColorTargetState>]| {
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(fragment),
            bind_group_layouts: &[Some(uniforms), Some(second_layout)],
            ..Default::default()
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(fragment),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(vertex),
                compilation_options: Default::default(),
                buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fragment),
                compilation_options: Default::default(),
                targets,
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
    let splat = make("vs_glow_splat", "fs_glow_splat", strip, &[GpuInstance::LAYOUT], &targets);
    let resolve = make(
        "vs_glow_resolve",
        "fs_glow_resolve",
        statistics,
        &[],
        &[Some(wgpu::ColorTargetState {
            format: target_format,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })],
    );
    (splat, resolve)
}

impl GlowTarget {
    pub(super) fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        compiled: &CompiledLatticeResources,
        pane: &PaneBuffers,
        strip: &InkStrip,
        has_light: bool,
    ) {
        let attachment = |view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        if has_light {
            let attachments = self.statistics.each_ref().map(attachment);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("directional_glow_splats"),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&compiled.glow_splat_pipeline);
            pass.set_bind_group(0, &pane.bind_group, &[]);
            pass.set_bind_group(1, &strip.blurred_bind_group, &[]);
            pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
            pass.draw(0..4, 0..pane.instance_count);
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("directional_glow_resolve"),
            color_attachments: &[attachment(&self.view)],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if has_light {
            pass.set_pipeline(&compiled.glow_resolve_pipeline);
            pass.set_bind_group(0, &pane.bind_group, &[]);
            pass.set_bind_group(1, &self.statistics_bind_group, &[]);
            pass.draw(0..4, 0..1);
        }
    }
}

impl LatticeCallback {
    pub(super) fn glow_draws(&self) -> bool {
        self.uniforms.glow.reach > 0.0
    }
}
