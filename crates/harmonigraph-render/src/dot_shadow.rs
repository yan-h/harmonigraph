//! Spectral-geometry shadows for the spiral's sounding-note dots.
//!
//! The sharp colored dots remain egui shapes. This callback is inserted just
//! before them and draws only their black knockout, so the pane keeps its
//! shape-level geometry tests and its bloom can still be laid over the fill.

use std::collections::HashMap;

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};

use crate::{create_vertex_buffer, wgpu, EGUI_BLEND};

pub(crate) const SRC: &str = include_str!("shaders/dot_shadow.wgsl");
const INITIAL_CAPACITY: usize = 16;
const PANE_TTL_PREPARES: u64 = 120;

#[cfg(any(test, feature = "hot-reload"))]
pub(crate) const ENTRY_POINTS: &[&str] =
    &["vs_dot_shadow", "vs_dot_shadow_cell", "fs_dot_shadow_coverage", "fs_dot_shadow"];

/// Draw the dark backing for the spiral's `dots` through the spectral geometry
/// style. The callback belongs immediately before the colored egui discs.
pub fn dot_shadow_paint_callback(
    rect: egui::Rect,
    dots: Vec<crate::GlowDot>,
    shadow: harmonigraph_scene::ShadowStyle,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
    shadow_surface_id: u64,
) -> egui::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(
        rect,
        DotShadowCallback { dots, shadow, target_format, pane_id, shadow_surface_id },
    )
}

struct DotShadowCallback {
    dots: Vec<crate::GlowDot>,
    shadow: harmonigraph_scene::ShadowStyle,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
    shadow_surface_id: u64,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Locals {
    screen_points: [f32; 2],
    shadow_atlas_size: [f32; 2],
    shadow: [f32; 4],
}

struct Resources {
    composite: wgpu::RenderPipeline,
    coverage: wgpu::RenderPipeline,
    locals_layout: wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    #[cfg(feature = "hot-reload")]
    generation: u64,
    panes: HashMap<u64, Pane>,
    prepares: u64,
}

struct Pane {
    locals: wgpu::Buffer,
    locals_bind: wgpu::BindGroup,
    dots: wgpu::Buffer,
    dot_capacity: usize,
    count: u32,
    last_seen: u64,
}

fn locals_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("spiral_dot_shadow_locals_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

fn pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layouts: &[Option<&wgpu::BindGroupLayout>],
    entries: (&str, &str),
    buffers: &[wgpu::VertexBufferLayout<'static>],
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    let (vertex, fragment) = entries;
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(fragment),
        bind_group_layouts: layouts,
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
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
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
    })
}

impl Resources {
    fn is_stale(&self, format: wgpu::TextureFormat) -> bool {
        #[cfg(feature = "hot-reload")]
        if self.generation != crate::reload::generation() {
            return true;
        }
        self.format != format
    }

    fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        shadow_layouts: &crate::spectral_shadow::Layouts,
    ) -> Self {
        let locals_layout = locals_layout(device);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spiral_dot_shadow_shader"),
            source: wgpu::ShaderSource::Wgsl(crate::dot_shadow_source().into()),
        });
        let composite = pipeline(
            device,
            &shader,
            &[
                Some(&locals_layout),
                None,
                Some(&shadow_layouts.atlas),
                Some(&shadow_layouts.casters),
            ],
            ("vs_dot_shadow", "fs_dot_shadow"),
            &[crate::GlowDot::LAYOUT],
            format,
            Some(EGUI_BLEND),
        );
        let coverage = pipeline(
            device,
            &shader,
            &[Some(&locals_layout)],
            ("vs_dot_shadow_cell", "fs_dot_shadow_coverage"),
            &[crate::GlowDot::LAYOUT, crate::shadow::ShadowBox::BESIDE_DOTS],
            crate::shadow::ATLAS_FORMAT,
            None,
        );
        Resources {
            composite,
            coverage,
            locals_layout,
            format,
            #[cfg(feature = "hot-reload")]
            generation: crate::reload::generation(),
            panes: HashMap::new(),
            prepares: 0,
        }
    }

    fn pane(&mut self, device: &wgpu::Device, pane_id: u64, prepares: u64) -> &mut Pane {
        let locals_layout = &self.locals_layout;
        let pane = self.panes.entry(pane_id).or_insert_with(|| {
            let locals = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("spiral_dot_shadow_locals"),
                size: std::mem::size_of::<Locals>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let locals_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("spiral_dot_shadow_locals_bind"),
                layout: locals_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: locals.as_entire_binding(),
                }],
            });
            Pane {
                locals,
                locals_bind,
                dots: create_vertex_buffer::<crate::GlowDot>(
                    device,
                    "spiral_shadow_dots",
                    INITIAL_CAPACITY,
                ),
                dot_capacity: INITIAL_CAPACITY,
                count: 0,
                last_seen: prepares,
            }
        });
        pane.last_seen = prepares;
        pane
    }
}

impl CallbackTrait for DotShadowCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let shadow_layouts = crate::spectral_shadow::layouts(device, callback_resources);
        let stale = callback_resources
            .get::<Resources>()
            .is_none_or(|resources| resources.is_stale(self.target_format));
        if stale {
            callback_resources.insert(Resources::new(device, self.target_format, &shadow_layouts));
        }
        let resources: &mut Resources = callback_resources.get_mut().expect("inserted above");
        resources.prepares = resources.prepares.wrapping_add(1);
        let prepares = resources.prepares;
        resources
            .panes
            .retain(|_, pane| prepares.saturating_sub(pane.last_seen) < PANE_TTL_PREPARES);
        let style = self.shadow.clamped();
        let ppp = screen.pixels_per_point.max(f32::EPSILON);
        let sigma = if style.casts() { crate::shadow::spectral_sigma_points(style) } else { 0.0 };
        let casters: Vec<_> = self
            .dots
            .iter()
            .map(|dot| crate::shadow::Caster {
                rect: [
                    dot.center[0] - dot.radius,
                    dot.center[1] - dot.radius,
                    2.0 * dot.radius,
                    2.0 * dot.radius,
                ],
                level: 0.75 * f32::from(dot.color[3]) / 255.0,
                sigma_points: sigma,
                kernel: style.kernel,
                direct_distance: true,
            })
            .collect();
        let locals = Locals {
            screen_points: [
                screen.size_in_pixels[0] as f32 / ppp,
                screen.size_in_pixels[1] as f32 / ppp,
            ],
            shadow_atlas_size: [1.0; 2],
            shadow: [
                sigma,
                style.depth,
                if style.kernel.is_distance() { crate::shadow::DISTANCE_KIND } else { 0.0 },
                crate::spectral_shadow_reach(style),
            ],
        };
        let coverage = resources.coverage.clone();
        let pane = resources.pane(device, self.pane_id, prepares);
        if self.dots.len() > pane.dot_capacity {
            pane.dot_capacity = self.dots.len().next_power_of_two();
            pane.dots = create_vertex_buffer::<crate::GlowDot>(
                device,
                "spiral_shadow_dots",
                pane.dot_capacity,
            );
        }
        pane.count = self.dots.len() as u32;
        if !self.dots.is_empty() {
            queue.write_buffer(&pane.dots, 0, bytemuck::cast_slice(&self.dots));
        }
        queue.write_buffer(&pane.locals, 0, bytemuck::bytes_of(&locals));
        let submission = crate::spectral_shadow::Submission {
            key: crate::spectral_shadow::ProducerKey::Dot(self.pane_id),
            casters,
            draw: crate::spectral_shadow::CellDraw::Dot {
                pipeline: coverage,
                locals: pane.locals_bind.clone(),
                dots: pane.dots.clone(),
                count: pane.count,
            },
            atlas_uniform: pane.locals.clone(),
            atlas_size_offset: std::mem::offset_of!(Locals, shadow_atlas_size) as u64,
        };
        crate::spectral_shadow::register(
            device,
            callback_resources,
            self.shadow_surface_id,
            submission,
        );
        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<Resources>() else {
            return;
        };
        let Some(pane) = resources.panes.get(&self.pane_id) else {
            return;
        };
        if pane.count == 0 {
            return;
        }
        pass.set_viewport(
            0.0,
            0.0,
            info.screen_size_px[0] as f32,
            info.screen_size_px[1] as f32,
            0.0,
            1.0,
        );
        let Some(shadow) = crate::spectral_shadow::binding(
            callback_resources,
            self.shadow_surface_id,
            crate::spectral_shadow::ProducerKey::Dot(self.pane_id),
        ) else {
            return;
        };
        if !shadow.active {
            return;
        }
        pass.set_pipeline(&resources.composite);
        pass.set_bind_group(0, &pane.locals_bind, &[]);
        pass.set_bind_group(2, shadow.atlas, &[]);
        pass.set_bind_group(3, shadow.casters, &[]);
        pass.set_vertex_buffer(0, pane.dots.slice(..));
        pass.draw(0..4, 0..pane.count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_harness::{headless_device, readback, render_to_texture};

    #[test]
    fn baked_dot_shadow_shader_validates() {
        let seam = crate::common_lines(crate::COMMON_SRC);
        crate::validate_wgsl("dot_shadow.wgsl", &crate::with_common(SRC), seam, ENTRY_POINTS)
            .expect("baked dot_shadow.wgsl must parse and validate");
    }

    #[test]
    fn either_spectral_geometry_endpoint_allocates_no_dot_shadow_atlas() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        for shadow in [
            harmonigraph_scene::ShadowStyle {
                width: 0.0,
                depth: 1.0,
                kernel: harmonigraph_scene::ShadowKernel::Gaussian,
            },
            harmonigraph_scene::ShadowStyle {
                width: 1.0,
                depth: 0.0,
                kernel: harmonigraph_scene::ShadowKernel::Gaussian,
            },
        ] {
            let cb = DotShadowCallback {
                dots: vec![crate::GlowDot { center: [32.0, 32.0], radius: 4.0, color: [255; 4] }],
                shadow,
                target_format: wgpu::TextureFormat::Rgba8Unorm,
                pane_id: 0,
                shadow_surface_id: 0,
            };
            let screen = ScreenDescriptor { size_in_pixels: [64, 64], pixels_per_point: 1.0 };
            let mut resources = CallbackResources::default();
            let mut encoder = device.create_command_encoder(&Default::default());
            cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
            crate::spectral_shadow::finish(
                &device,
                &queue,
                &screen,
                &mut encoder,
                &mut resources,
                0,
            );
            assert!(
                !crate::spectral_shadow::target_allocated(&resources, 0),
                "{shadow:?} allocated a shadow atlas"
            );
        }
    }

    #[test]
    fn both_spiral_geometry_kernels_hold_at_editor_and_export_scales() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        for kernel in
            [harmonigraph_scene::ShadowKernel::Distance, harmonigraph_scene::ShadowKernel::Gaussian]
        {
            for ppp in [1.0f32, 1.5, 2.0, 4.0] {
                let side = (64.0 * ppp) as u32;
                let size = [side.div_ceil(64) * 64, side];
                let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(64.0, 64.0));
                let cb = DotShadowCallback {
                    dots: vec![crate::GlowDot {
                        center: [32.0, 32.0],
                        radius: 4.0,
                        color: [255; 4],
                    }],
                    shadow: harmonigraph_scene::ShadowStyle { width: 0.5, depth: 1.0, kernel },
                    target_format: wgpu::TextureFormat::Rgba8Unorm,
                    pane_id: 0,
                    shadow_surface_id: 0,
                };
                let screen = ScreenDescriptor { size_in_pixels: size, pixels_per_point: ppp };
                let mut resources = CallbackResources::default();
                let mut encoder = device.create_command_encoder(&Default::default());
                let buffers = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
                crate::spectral_shadow::finish(
                    &device,
                    &queue,
                    &screen,
                    &mut encoder,
                    &mut resources,
                    0,
                );
                queue.submit(buffers.into_iter().chain([encoder.finish()]));
                let texture = render_to_texture(
                    &device,
                    &queue,
                    size,
                    wgpu::TextureFormat::Rgba8Unorm,
                    wgpu::Color::WHITE,
                    |pass| {
                        cb.paint(
                            egui::PaintCallbackInfo {
                                viewport: rect,
                                clip_rect: rect,
                                pixels_per_point: ppp,
                                screen_size_px: size,
                            },
                            pass,
                            &resources,
                        );
                    },
                );
                let frame = readback(&device, &queue, &texture, size);
                let pixel = |x: f32, y: f32| {
                    let x = (x * ppp) as u32;
                    let y = (y * ppp) as u32;
                    let i = ((y * size[0] + x) * 4) as usize;
                    [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
                };
                assert!(pixel(38.0, 32.0)[0] < 250, "{kernel:?} at {ppp} ppp cast no edge");
                assert_eq!(pixel(4.0, 4.0), [255; 4], "{kernel:?} at {ppp} ppp escaped");
            }
        }
    }
}
