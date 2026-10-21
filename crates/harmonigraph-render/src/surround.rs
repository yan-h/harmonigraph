//! The BACKDROP CUT round a note (#1525, #1528), shared by the two producers
//! that draw one — the roll's ribbons and the Spiral's dots — so both panes'
//! notes wear one surround on one set of dials.
//!
//! A producer draws its notes once in `prepare`, into one target over its own
//! rect ([`SurroundTargets`]): under a depth test the strongest note's cut
//! lands there with its fill color. `paint` then lays it over the picture in
//! one draw ([`SurroundComposite`]). What a note's cut IS at a fragment — its
//! distance, what holds it out — is the producer's own shader; the fade's
//! shape and how the cut lands are `shaders/surround.wgsl`, which every
//! producer joins.

use crate::uniforms::{uniform_group, Float4};
use crate::{wgpu, EGUI_BLEND};

/// The cut's arithmetic: functions only, joined to each producer's module and
/// to the composite's.
pub(crate) const SRC: &str = include_str!("shaders/surround.wgsl");
const COMPOSITE_SRC: &str = include_str!("shaders/surround_composite.wgsl");

#[cfg(test)]
pub(crate) const COMPOSITE_ENTRY_POINTS: &[&str] =
    &["vs_cover", "fs_surround_gamma", "fs_surround_linear"];

/// The composite's module: the cut's arithmetic, then the draw that lays it.
pub(crate) fn composite_source() -> String {
    format!("{SRC}\n{COMPOSITE_SRC}")
}

/// The cut as the pane dials it: the notes' surround takes the picture away
/// next to each note — whole at its edge at a full `amount`, fading out over
/// `reach` along a curve `shape` bends — and fills what it took with the
/// note's own hue. Over black it is that hue laid on.
///
/// Where two notes' cuts meet, the stronger is what lands, with its color, so a
/// crowd of notes is no more cut between them than one note is.
///
/// `amount` is a 0..1 share; [`SurroundCut::sanitized`] clamps
/// every field, and the panes hand these over as the user dialled them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurroundCut {
    /// How much of the picture the cut takes at a note's edge. 0 is no
    /// surround at all.
    pub amount: f32,
    /// How far past a note's edge the cut fades out, in points
    /// ([`SURROUND_REACH_RANGE`]).
    pub reach: f32,
    /// The fade's shape ([`SURROUND_SHAPE_RANGE`]): 0 a straight line, -1 a
    /// steep drop at the note and a long soft tail, 1 held near the note and a
    /// steep drop at the far end.
    pub shape: f32,
}

/// The cut's reach, in points, that [`SurroundCut::reach`] is held to: the
/// range the earlier `1..=8` σ of the Notes shadow's default σ (3.75 points)
/// spanned.
pub const SURROUND_REACH_RANGE: std::ops::RangeInclusive<f32> = 3.75..=30.0;

/// [`SurroundCut::shape`]'s range: a bipolar bend about the straight line.
pub const SURROUND_SHAPE_RANGE: std::ops::RangeInclusive<f32> = -1.0..=1.0;

/// How far [`SurroundCut::shape`] moves Schlick's bias parameter `a` off its
/// straight line at 0.5: at either end of the bar it stands at 0.05 or 0.95,
/// a `k` of 18 or about -0.95 (`surround_ramp` in surround.wgsl). Short of the
/// degenerate 0 and 1, where the fade would be a step.
const SURROUND_SHAPE_BIAS: f32 = 0.45;

/// The bias `k` that `surround_ramp` in surround.wgsl bends the fade by at
/// [`SurroundCut::shape`] `shape`: Schlick's `1 / a - 2`.
fn shape_bias(shape: f32) -> f32 {
    let a = 0.5 - SURROUND_SHAPE_BIAS * shape.clamp(-1.0, 1.0);
    1.0 / a - 2.0
}

/// How much of its amount the cut still takes `x` of the way out to its reach
/// (0..1), at a [`SurroundCut::shape`] of `shape`: `surround_ramp` in
/// surround.wgsl, for a pane that draws the fade it dials.
pub fn surround_fade(shape: f32, x: f32) -> f32 {
    let k = shape_bias(shape);
    let t = x.clamp(0.0, 1.0);
    1.0 - t / (k * (1.0 - t) + 1.0)
}

impl SurroundCut {
    /// No surround: what a cut at its defaults but for its amount draws.
    pub const OFF: Self = Self { amount: 0.0, reach: 15.0, shape: 0.0 };

    /// Every field in its range, a non-finite one taken as [`Self::OFF`]'s.
    pub fn sanitized(self) -> Self {
        let off = Self::OFF;
        let within = |v: f32, off: f32, range: std::ops::RangeInclusive<f32>| {
            if v.is_finite() { v } else { off }.clamp(*range.start(), *range.end())
        };
        Self {
            amount: within(self.amount, off.amount, 0.0..=1.0),
            reach: within(self.reach, off.reach, SURROUND_REACH_RANGE),
            shape: within(self.shape, off.shape, SURROUND_SHAPE_RANGE),
        }
    }

    /// How far the surround paints past a note's edge, in points of a picture
    /// drawn at `point_scale`, and 0 when it draws nothing. What a producer's
    /// quads are grown by, and what the roll's culls keep a leaving note alive
    /// for.
    pub fn reach_points(self, point_scale: f32) -> f32 {
        let cut = self.sanitized();
        if cut.amount > 0.0 {
            cut.reach * point_scale.max(0.0)
        } else {
            0.0
        }
    }

    /// The cut as surround.wgsl reads it, at `point_scale`: all 0 while it
    /// draws nothing.
    pub(crate) fn uniform(self, point_scale: f32) -> SurroundUniform {
        let cut = self.sanitized();
        let reach = cut.reach_points(point_scale);
        if reach <= 0.0 {
            return SurroundUniform { cut: Float4([0.0; 4]) };
        }
        SurroundUniform { cut: Float4([cut.amount, reach, shape_bias(cut.shape), 0.0]) }
    }
}

uniform_group! {
/// `SurroundCut` in surround.wgsl: [`SurroundCut::uniform`]'s four.
struct SurroundUniform {
    cut: Float4,
}
}

impl SurroundUniform {
    /// Whether the cut draws: what a producer keeps its targets for.
    pub(crate) fn draws(&self) -> bool {
        self.cut.0[0] > 0.0 && self.cut.0[1] > 0.0
    }
}

/// The strongest note's straight gamma-space fill color, and in alpha how much
/// of the picture its cut takes, the amount included. Eight bits, as the surface it lands on: what it
/// holds is the color and coverage of a surround laid over an eight-bit
/// picture, so all it costs is one rounding at the composite.
const SURROUND_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// `1 - cut` per texel, under a `Less` test: the strongest note's fragment is
/// the one that lands, its color with it. A `max` blend would keep the
/// strongest cut too, but take each color channel's max separately — a hue
/// neither note has, wherever two cuts meet. Sixteen bits resolves the cut far
/// finer than the eight the color's alpha keeps.
const SURROUND_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth16Unorm;

/// The composite that lays a producer's [`SurroundTargets`] over the picture,
/// premultiplied, in the shading the target wants; and the layout its targets
/// bind through.
pub(crate) struct SurroundComposite {
    pipeline: wgpu::RenderPipeline,
    read_layout: wgpu::BindGroupLayout,
}

impl SurroundComposite {
    pub(crate) fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let read_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("surround_read_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("surround_composite_shader"),
            source: wgpu::ShaderSource::Wgsl(composite_source().into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("surround_composite_layout"),
            bind_group_layouts: &[Some(&read_layout)],
            ..Default::default()
        });
        let entry_point =
            if target_format.is_srgb() { "fs_surround_linear" } else { "fs_surround_gamma" };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("surround_composite"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_cover"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(entry_point),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(EGUI_BLEND),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        SurroundComposite { pipeline, read_layout }
    }
}

/// The entry points and vertex buffers a producer's surround pipeline is built
/// from: its own module, which has joined surround.wgsl, with its own vertex
/// stage and bind groups. The fragment writes the note's straight fill color
/// with its cut in alpha, and `1 - cut` as depth.
pub(crate) struct SurroundStages<'a> {
    pub(crate) label: &'a str,
    pub(crate) vertex: &'a str,
    pub(crate) fragment: &'a str,
    pub(crate) buffers: &'a [wgpu::VertexBufferLayout<'a>],
}

/// A producer's notes into its [`SurroundTargets`], the strongest kept with its
/// color.
pub(crate) fn surround_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layouts: &[Option<&wgpu::BindGroupLayout>],
    stages: SurroundStages<'_>,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(stages.label),
        bind_group_layouts: layouts,
        ..Default::default()
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(stages.label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(stages.vertex),
            compilation_options: Default::default(),
            buffers: stages.buffers,
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(stages.fragment),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: SURROUND_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: SURROUND_DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// A producer's surround over its rect, one texel per device pixel: the
/// strongest note's color and cut, and the depth that picks it. Held while the
/// cut draws, and dropped with it; [`ensure`](Self::ensure) rebuilds it when
/// the rect resizes.
pub(crate) struct SurroundTargets {
    view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    read_bind_group: wgpu::BindGroup,
    /// The rect's size in device pixels these were built for.
    size: [u32; 2],
}

impl SurroundTargets {
    pub(crate) fn new(
        device: &wgpu::Device,
        composite: &SurroundComposite,
        size: [u32; 2],
    ) -> Self {
        let target = |label, format, usage| {
            crate::gpu_memory::texture(
                device,
                &wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                },
            )
            .create_view(&Default::default())
        };
        let view = target(
            "surround",
            SURROUND_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let depth_view =
            target("surround_depth", SURROUND_DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT);
        SurroundTargets {
            read_bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("surround_read_bind_group"),
                layout: &composite.read_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                }],
            }),
            view,
            depth_view,
            size,
        }
    }

    /// `slot`'s targets, built for a rect `size` device pixels across: kept
    /// while they are that size, and rebuilt when it is not.
    pub(crate) fn ensure<'a>(
        slot: &'a mut Option<Self>,
        device: &wgpu::Device,
        composite: &SurroundComposite,
        size: [u32; 2],
    ) -> &'a Self {
        if slot.as_ref().is_some_and(|targets| targets.size != size) {
            *slot = None;
        }
        slot.get_or_insert_with(|| Self::new(device, composite, size))
    }

    /// The notes' surround into these targets. `draw` binds the producer's own
    /// groups and vertex buffers and draws its notes, after the pipeline is
    /// set.
    pub(crate) fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        draw: impl FnOnce(&mut wgpu::RenderPass<'_>),
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("surround"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            // Cleared to 1, the depth of no cut at all, which no fragment of no
            // cut can beat.
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        draw(&mut pass);
    }

    /// Lay the surround over the picture, in the viewport the caller set to the
    /// rect these targets cover.
    pub(crate) fn paint(
        &self,
        pass: &mut wgpu::RenderPass<'static>,
        composite: &SurroundComposite,
    ) {
        pass.set_pipeline(&composite.pipeline);
        pass.set_bind_group(0, &self.read_bind_group, &[]);
        pass.draw(0..4, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baked_composite_shader_validates() {
        crate::validate_wgsl(
            "surround_composite.wgsl",
            &composite_source(),
            0,
            COMPOSITE_ENTRY_POINTS,
        )
        .expect("baked surround_composite.wgsl must parse and validate");
    }
}
