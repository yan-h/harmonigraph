//! One shadow atlas and one optional blur chain for each spectral surface.
//!
//! The roll and spiral dots own their analytic fields, while text owns its
//! glyph field. Their callbacks therefore register the draw that can fill
//! their cells here. A final, paint-less callback runs after all producers
//! have prepared, packs every group together, fills the common atlas and
//! sweeps its Gaussian cells once in each direction. The visible callbacks
//! then composite from that shared result in their original paint order.

use std::collections::HashMap;

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};

use crate::{create_vertex_buffer, shadow, wgpu};

const INITIAL_CAPACITY: usize = 16;
const SURFACE_TTL_PASSES: u64 = 120;

#[derive(Clone)]
pub(crate) struct Layouts {
    pub atlas: wgpu::BindGroupLayout,
    pub casters: wgpu::BindGroupLayout,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum ProducerKey {
    Roll(u64),
    Text(u64),
    Dot(u64),
}

impl ProducerKey {
    fn order(self) -> u8 {
        match self {
            Self::Roll(_) | Self::Dot(_) => 0,
            Self::Text(_) => 1,
        }
    }
}

pub(crate) enum CellDraw {
    Roll {
        pipeline: wgpu::RenderPipeline,
        locals: wgpu::BindGroup,
        instances: wgpu::Buffer,
        count: u32,
    },
    Text {
        coverage: wgpu::RenderPipeline,
        distance: wgpu::RenderPipeline,
        distance_pad: wgpu::RenderPipeline,
        locals: wgpu::BindGroup,
        glyphs: wgpu::Buffer,
        count: u32,
        kernel: harmonigraph_scene::ShadowKernel,
    },
    Dot {
        pipeline: wgpu::RenderPipeline,
        locals: wgpu::BindGroup,
        dots: wgpu::Buffer,
        count: u32,
    },
    #[cfg(test)]
    None,
}

pub(crate) struct Submission {
    pub key: ProducerKey,
    pub casters: Vec<shadow::Caster>,
    pub draw: CellDraw,
    /// The producer's uniform block and the byte offset of its atlas size.
    pub atlas_uniform: wgpu::Buffer,
    pub atlas_size_offset: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct ProducerRange {
    start: u32,
    count: u32,
    active: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScheduleStats {
    pub blur_chains: u32,
    /// Bit zero is Distance and bit one is Gaussian.
    pub renderer_kinds: u8,
    pub groups: u32,
}

struct Surface {
    boxes: wgpu::Buffer,
    box_capacity: usize,
    casters: wgpu::Buffer,
    caster_bind: wgpu::BindGroup,
    caster_capacity: usize,
    ranges: HashMap<ProducerKey, ProducerRange>,
    submissions: Vec<Submission>,
    target: Option<shadow::ShadowTarget>,
    stats: ScheduleStats,
    last_seen_pass: u64,
}

pub(crate) struct Resources {
    layouts: Layouts,
    sampler: wgpu::Sampler,
    dummy: shadow::ShadowTarget,
    cells: shadow::CellPipelines,
    surfaces: HashMap<u64, Surface>,
}

impl Resources {
    fn new(device: &wgpu::Device) -> Self {
        let atlas = shadow::read_layout(device);
        let casters = shadow::caster_layout(device);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("spectral_shadow_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let dummy = shadow::ShadowTarget::new(device, &atlas, &sampler, [1, 1]);
        let cells = shadow::create_cell_pipelines(device, &atlas);
        Resources {
            layouts: Layouts { atlas, casters },
            sampler,
            dummy,
            cells,
            surfaces: HashMap::new(),
        }
    }

    fn surface(&mut self, device: &wgpu::Device, id: u64, pass_nr: u64) -> &mut Surface {
        let caster_layout = &self.layouts.casters;
        let surface = self.surfaces.entry(id).or_insert_with(|| {
            let (casters, caster_bind) =
                shadow::caster_buffer(device, caster_layout, INITIAL_CAPACITY);
            Surface {
                boxes: create_vertex_buffer::<shadow::ShadowBox>(
                    device,
                    "spectral_shadow_boxes",
                    INITIAL_CAPACITY,
                ),
                box_capacity: INITIAL_CAPACITY,
                casters,
                caster_bind,
                caster_capacity: INITIAL_CAPACITY,
                ranges: HashMap::new(),
                submissions: Vec::new(),
                target: None,
                stats: ScheduleStats::default(),
                last_seen_pass: pass_nr,
            }
        });
        surface.last_seen_pass = pass_nr;
        surface
    }
}

pub(crate) fn layouts(
    device: &wgpu::Device,
    callback_resources: &mut CallbackResources,
) -> Layouts {
    if callback_resources.get::<Resources>().is_none() {
        callback_resources.insert(Resources::new(device));
    }
    callback_resources.get::<Resources>().expect("inserted above").layouts.clone()
}

pub(crate) fn register_for_pass(
    device: &wgpu::Device,
    callback_resources: &mut CallbackResources,
    surface_id: u64,
    submission: Submission,
    pass_nr: u64,
) {
    if callback_resources.get::<Resources>().is_none() {
        callback_resources.insert(Resources::new(device));
    }
    let resources: &mut Resources = callback_resources.get_mut().expect("inserted above");
    let surface = resources.surface(device, surface_id, pass_nr);
    surface.submissions.retain(|old| old.key != submission.key);
    surface.submissions.push(submission);
}

#[cfg(test)]
pub(crate) fn register(
    device: &wgpu::Device,
    callback_resources: &mut CallbackResources,
    surface_id: u64,
    submission: Submission,
) {
    register_for_pass(device, callback_resources, surface_id, submission, 0);
}

pub(crate) struct Binding<'a> {
    pub atlas: &'a wgpu::BindGroup,
    pub casters: &'a wgpu::BindGroup,
    pub boxes: &'a wgpu::Buffer,
    pub start: u32,
    pub count: u32,
    pub active: bool,
}

pub(crate) fn binding<'a>(
    callback_resources: &'a CallbackResources,
    surface_id: u64,
    key: ProducerKey,
) -> Option<Binding<'a>> {
    let resources = callback_resources.get::<Resources>()?;
    let surface = resources.surfaces.get(&surface_id)?;
    let range = *surface.ranges.get(&key)?;
    Some(Binding {
        atlas: surface.target.as_ref().unwrap_or(&resources.dummy).read(),
        casters: &surface.caster_bind,
        boxes: &surface.boxes,
        start: range.start,
        count: range.count,
        active: range.active,
    })
}

fn box_slice(buffer: &wgpu::Buffer, range: ProducerRange) -> wgpu::BufferSlice<'_> {
    let stride = std::mem::size_of::<shadow::ShadowBox>() as u64;
    buffer.slice(stride * u64::from(range.start)..stride * u64::from(range.start + range.count))
}

fn finish_for_pass(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    screen: &ScreenDescriptor,
    encoder: &mut wgpu::CommandEncoder,
    callback_resources: &mut CallbackResources,
    surface_id: u64,
    pass_nr: u64,
) {
    if callback_resources.get::<Resources>().is_none() {
        callback_resources.insert(Resources::new(device));
    }
    let resources: &mut Resources = callback_resources.get_mut().expect("inserted above");
    resources
        .surfaces
        .retain(|_, surface| pass_nr.saturating_sub(surface.last_seen_pass) < SURFACE_TTL_PASSES);

    let layouts = resources.layouts.clone();
    let sampler = resources.sampler.clone();
    let blur_x = resources.cells.blur_x.clone();
    let blur_y = resources.cells.blur_y.clone();
    let surface = resources.surface(device, surface_id, pass_nr);
    surface.submissions.sort_by_key(|submission| submission.key.order());

    let mut casters = Vec::new();
    let mut starts = Vec::with_capacity(surface.submissions.len());
    for submission in &surface.submissions {
        starts.push(casters.len() as u32);
        casters.extend_from_slice(&submission.casters);
    }
    let packed = shadow::pack(
        &casters,
        screen.pixels_per_point.max(f32::EPSILON),
        device.limits().max_texture_dimension_2d,
    );
    let has_cells = packed.boxes.iter().any(|b| b.cell[2] > 0.0 && b.cell[3] > 0.0);
    let has_gaussian = packed
        .boxes
        .iter()
        .any(|b| b.cell[2] > 0.0 && b.cell[3] > 0.0 && b.who[1] < 0.5 * shadow::DISTANCE_KIND);

    if packed.boxes.len() > surface.box_capacity {
        surface.box_capacity = packed.boxes.len().next_power_of_two();
        surface.boxes = create_vertex_buffer::<shadow::ShadowBox>(
            device,
            "spectral_shadow_boxes",
            surface.box_capacity,
        );
    }
    if packed.casters.len() > surface.caster_capacity {
        surface.caster_capacity = packed.casters.len().next_power_of_two();
        (surface.casters, surface.caster_bind) =
            shadow::caster_buffer(device, &layouts.casters, surface.caster_capacity);
    }
    if !packed.boxes.is_empty() {
        queue.write_buffer(&surface.boxes, 0, bytemuck::cast_slice(&packed.boxes));
        queue.write_buffer(&surface.casters, 0, bytemuck::cast_slice(&packed.casters));
    }

    surface.ranges.clear();
    let mut renderer_kinds = 0;
    for (submission, start) in surface.submissions.iter().zip(starts) {
        let count = submission.casters.len() as u32;
        let active = packed.casters[start as usize..(start + count) as usize]
            .iter()
            .any(|caster| caster.shade[0] > 0.0);
        for caster in &packed.casters[start as usize..(start + count) as usize] {
            if caster.shade[0] > 0.0 {
                renderer_kinds |=
                    if caster.shade[1] >= 0.5 * shadow::DISTANCE_KIND { 1 } else { 2 };
            }
        }
        surface.ranges.insert(submission.key, ProducerRange { start, count, active });
    }
    surface.stats =
        ScheduleStats { blur_chains: 0, renderer_kinds, groups: surface.submissions.len() as u32 };

    if has_cells {
        if surface.target.as_ref().is_none_or(|target| !target.holds(packed.size)) {
            let held = surface.target.as_ref().map_or([0, 0], |target| target.size);
            surface.target = Some(shadow::ShadowTarget::new(
                device,
                &layouts.atlas,
                &sampler,
                [packed.size[0].max(held[0]), packed.size[1].max(held[1])],
            ));
        }
        let target = surface.target.as_mut().expect("created above");
        target.ensure_half(device, &layouts.atlas, &sampler, has_gaussian);
    } else {
        surface.target = None;
    }

    let atlas_size =
        surface.target.as_ref().map_or([1.0, 1.0], |target| target.size.map(|v| v as f32));
    for submission in &surface.submissions {
        queue.write_buffer(
            &submission.atlas_uniform,
            submission.atlas_size_offset,
            bytemuck::cast_slice(&atlas_size),
        );
    }

    if let Some(target) = surface.target.as_ref() {
        {
            let mut pass = target.ink_pass(encoder);
            for submission in &surface.submissions {
                let range = surface.ranges[&submission.key];
                if range.count == 0 {
                    continue;
                }
                let boxes = box_slice(&surface.boxes, range);
                match &submission.draw {
                    CellDraw::Roll { pipeline, locals, instances, count } => {
                        pass.set_pipeline(pipeline);
                        pass.set_bind_group(0, locals, &[]);
                        pass.set_vertex_buffer(0, instances.slice(..));
                        pass.set_vertex_buffer(1, boxes);
                        pass.draw(0..4, 0..*count);
                    }
                    CellDraw::Text {
                        coverage,
                        distance,
                        distance_pad,
                        locals,
                        glyphs,
                        count,
                        kernel,
                    } => {
                        pass.set_bind_group(0, locals, &[]);
                        if kernel.is_distance() {
                            pass.set_pipeline(distance_pad);
                            pass.set_vertex_buffer(0, boxes);
                            pass.draw(0..4, 0..range.count);
                            pass.set_pipeline(distance);
                        } else {
                            pass.set_pipeline(coverage);
                        }
                        pass.set_vertex_buffer(0, glyphs.slice(..));
                        pass.set_vertex_buffer(1, boxes);
                        pass.draw(0..4, 0..*count);
                    }
                    CellDraw::Dot { pipeline, locals, dots, count } => {
                        pass.set_pipeline(pipeline);
                        pass.set_bind_group(0, locals, &[]);
                        pass.set_vertex_buffer(0, dots.slice(..));
                        pass.set_vertex_buffer(1, boxes);
                        pass.draw(0..4, 0..*count);
                    }
                    #[cfg(test)]
                    CellDraw::None => {}
                }
            }
        }
        if has_gaussian {
            target.blur(encoder, (&blur_x, &blur_y), &surface.boxes, packed.boxes.len() as u32);
            surface.stats.blur_chains += 1;
        }
    }
    surface.submissions.clear();
}

#[cfg(test)]
pub(crate) fn finish(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    screen: &ScreenDescriptor,
    encoder: &mut wgpu::CommandEncoder,
    callback_resources: &mut CallbackResources,
    surface_id: u64,
) {
    finish_for_pass(device, queue, screen, encoder, callback_resources, surface_id, 0);
}

/// Finish the shared shadow field for one spectral or spiral surface.
///
/// This callback paints nothing. Its position after the surface's producers
/// is significant only because egui prepares callbacks in paint order;
/// `pass_nr` is the painter context's cumulative pass number.
pub fn spectral_shadow_prepare_callback(
    rect: egui::Rect,
    surface_id: u64,
    pass_nr: u64,
) -> egui::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(rect, FinishCallback { surface_id, pass_nr })
}

struct FinishCallback {
    surface_id: u64,
    pass_nr: u64,
}

impl CallbackTrait for FinishCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        finish_for_pass(
            device,
            queue,
            screen,
            encoder,
            callback_resources,
            self.surface_id,
            self.pass_nr,
        );
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        _render_pass: &mut wgpu::RenderPass<'static>,
        _callback_resources: &CallbackResources,
    ) {
    }
}

#[cfg(test)]
pub(crate) fn stats(resources: &CallbackResources, surface_id: u64) -> ScheduleStats {
    resources
        .get::<Resources>()
        .and_then(|resources| resources.surfaces.get(&surface_id))
        .map_or(ScheduleStats::default(), |surface| surface.stats)
}

#[cfg(test)]
pub(crate) fn target_allocated(resources: &CallbackResources, surface_id: u64) -> bool {
    resources
        .get::<Resources>()
        .and_then(|resources| resources.surfaces.get(&surface_id))
        .is_some_and(|surface| surface.target.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_harness::headless_device;

    fn submit(
        device: &wgpu::Device,
        resources: &mut CallbackResources,
        key: ProducerKey,
        kernel: harmonigraph_scene::ShadowKernel,
    ) {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("spectral_shadow_schedule_test_uniform"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        register(
            device,
            resources,
            7,
            Submission {
                key,
                casters: vec![shadow::Caster {
                    rect: [8.0, 8.0, 12.0, 12.0],
                    level: 1.0,
                    sigma_points: 2.0,
                    kernel,
                    direct_distance: false,
                }],
                draw: CellDraw::None,
                atlas_uniform: uniform,
                atlas_size_offset: 0,
            },
        );
    }

    #[test]
    fn two_gaussian_groups_dispatch_one_blur_chain_and_mixed_dispatches_two_kinds() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let screen = ScreenDescriptor { size_in_pixels: [64, 64], pixels_per_point: 1.0 };
        let mut resources = CallbackResources::default();

        submit(
            &device,
            &mut resources,
            ProducerKey::Roll(1),
            harmonigraph_scene::ShadowKernel::Gaussian,
        );
        submit(
            &device,
            &mut resources,
            ProducerKey::Text(2),
            harmonigraph_scene::ShadowKernel::Gaussian,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        finish(&device, &queue, &screen, &mut encoder, &mut resources, 7);
        queue.submit([encoder.finish()]);
        assert_eq!(
            stats(&resources, 7),
            ScheduleStats { blur_chains: 1, renderer_kinds: 2, groups: 2 }
        );

        submit(
            &device,
            &mut resources,
            ProducerKey::Roll(1),
            harmonigraph_scene::ShadowKernel::Distance,
        );
        submit(
            &device,
            &mut resources,
            ProducerKey::Text(2),
            harmonigraph_scene::ShadowKernel::Gaussian,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        finish(&device, &queue, &screen, &mut encoder, &mut resources, 7);
        queue.submit([encoder.finish()]);
        assert_eq!(
            stats(&resources, 7),
            ScheduleStats { blur_chains: 1, renderer_kinds: 3, groups: 2 }
        );

        for surface in [11, 12] {
            let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("spectral_shadow_second_surface_uniform"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            for key in [ProducerKey::Roll(1), ProducerKey::Text(2)] {
                register(
                    &device,
                    &mut resources,
                    surface,
                    Submission {
                        key,
                        casters: vec![shadow::Caster {
                            rect: [8.0, 8.0, 12.0, 12.0],
                            level: 1.0,
                            sigma_points: 2.0,
                            kernel: harmonigraph_scene::ShadowKernel::Gaussian,
                            direct_distance: false,
                        }],
                        draw: CellDraw::None,
                        atlas_uniform: uniform.clone(),
                        atlas_size_offset: 0,
                    },
                );
            }
            let mut encoder = device.create_command_encoder(&Default::default());
            finish(&device, &queue, &screen, &mut encoder, &mut resources, surface);
            queue.submit([encoder.finish()]);
            assert_eq!(stats(&resources, surface).blur_chains, 1);
        }
    }
}
