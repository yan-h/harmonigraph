//! The BACKDROP CUT round a note (#1525, #1528), shared by the two producers
//! that draw one — the roll's ribbons and the Spiral's dots — so both panes'
//! notes wear one surround on one set of dials.
//!
//! A producer draws its notes twice in `prepare`, into two targets over its
//! own rect ([`SurroundTargets`]): each note's cut, summed for the halo blend
//! or kept strongest-wins, and, only while the cut fills, the strongest note's
//! fill color. `paint` then lays both over the picture in one draw
//! ([`SurroundComposite`]). What a note's cut IS at a fragment — its distance,
//! what holds it out — is the producer's own shader; how cuts combine and how
//! they land is `shaders/surround.wgsl`, which every producer joins.

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
/// note's own hue as far as `fill` says. Over black it draws the fill alone.
///
/// Neighbouring notes' cuts combine as a smooth minimum of their distances,
/// softened by `blend` (0 the plain strongest-wins, which folds into a crease
/// along the line between two notes once the cut is wide and soft).
///
/// Every field is a 0..1 share but `reach` and `shape`;
/// [`SurroundCut::sanitized`] clamps them, and the panes hand these over as the
/// user dialled them.
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
    /// How much of what the cut takes is filled with the note's own hue rather
    /// than left black.
    pub fill: f32,
    /// How softly neighbouring notes' cuts merge: a smooth minimum of their
    /// distances, softened over `blend` × [`SURROUND_HALO_BLEND_MAX`] of the
    /// reach, and the plain minimum at 0.
    pub blend: f32,
}

/// The cut's reach, in points, that [`SurroundCut::reach`] is held to: the
/// range the earlier `1..=8` σ of the Notes shadow's default σ (3.75 points)
/// spanned.
pub const SURROUND_REACH_RANGE: std::ops::RangeInclusive<f32> = 3.75..=30.0;

/// [`SurroundCut::shape`]'s range: a bipolar bend about the straight line.
pub const SURROUND_SHAPE_RANGE: std::ops::RangeInclusive<f32> = -1.0..=1.0;

/// The halo blend's softness at a [`SurroundCut::blend`] of 1, as a share of
/// the reach: the smooth minimum's `k` over the fade's length.
///
/// A share of the reach because the crease the blend takes out widens with the
/// cut: a fade twice as long folds over twice the distance and wants twice the
/// softening. Held to a quarter so a note's share of the blend, which is
/// lowered by its value at the reach (`surround_share` in surround.wgsl),
/// keeps that value — and the slope it steps by there — under e⁻⁴ of a share
/// at the edge.
pub const SURROUND_HALO_BLEND_MAX: f32 = 0.25;

/// The smallest halo blend softness summed as a smooth minimum, as a share of
/// the reach. Every share is `exp(-x / κ)` for an `x` up to 1, and at this κ
/// the least of them, e⁻⁶⁴, is still a normal f32 with room for the sum's
/// rounding and any flush of denormals. Below it the cut is the plain
/// strongest-wins one, which differs from the smooth minimum there by at most
/// κ ln 2, a hundredth of the reach.
const SURROUND_KAPPA_MIN: f32 = 1.0 / 64.0;

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
    pub const OFF: Self = Self { amount: 0.0, reach: 15.0, shape: 0.0, fill: 0.0, blend: 1.0 };

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
            fill: within(self.fill, off.fill, 0.0..=1.0),
            blend: within(self.blend, off.blend, 0.0..=1.0),
        }
    }

    /// Whether this cut draws anything.
    pub fn draws(self) -> bool {
        self.sanitized().amount > 0.0
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
            return SurroundUniform { cut: Float4([0.0; 4]), fill: Float4([0.0; 4]) };
        }
        let kappa = cut.blend * SURROUND_HALO_BLEND_MAX;
        SurroundUniform {
            cut: Float4([
                cut.amount,
                reach,
                shape_bias(cut.shape),
                if kappa >= SURROUND_KAPPA_MIN { kappa } else { 0.0 },
            ]),
            fill: Float4([cut.fill, 0.0, 0.0, 0.0]),
        }
    }
}

uniform_group! {
/// `SurroundCut` in surround.wgsl: [`SurroundCut::uniform`]'s eight.
struct SurroundUniform {
    cut: Float4,
    fill: Float4,
}
}

impl SurroundUniform {
    /// Whether the cut draws: what a producer keeps its targets for.
    pub(crate) fn draws(&self) -> bool {
        self.cut.0[0] > 0.0 && self.cut.0[1] > 0.0
    }

    /// Whether the cut fills, which is when the color target is drawn.
    fn fills(&self) -> bool {
        self.fill.0[0] > 0.0
    }

    /// Whether the cut target sums a smooth minimum rather than keeping the
    /// strongest note's cut.
    fn blends(&self) -> bool {
        self.cut.0[3] > 0.0
    }
}

/// What each note's [`surround_share`](../shaders/surround.wgsl) lands in:
/// summed or kept strongest-wins, in f32 so the halo blend's sum keeps its
/// precision out to the reach (see [`SURROUND_KAPPA_MIN`]). Blending it needs
/// `FLOAT32_BLENDABLE`, which `crate::device_descriptor` requires.
const CUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// The strongest note's fill color: straight gamma-space color, as the
/// eight-bit surface it lands on holds it, so all it costs is one rounding.
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// `1 - cut` per color texel, under a `Less` test: the strongest note's
/// fragment is the one that lands, its color with it. A `max` blend would keep
/// each channel's max separately — a hue neither note has, wherever two cuts
/// meet.
const COLOR_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth16Unorm;

/// The composite that lays a producer's [`SurroundTargets`] over the picture,
/// premultiplied, in the shading the target wants; and the two layouts its
/// targets bind through.
pub(crate) struct SurroundComposite {
    pipeline: wgpu::RenderPipeline,
    uniform_layout: wgpu::BindGroupLayout,
    read_layout: wgpu::BindGroupLayout,
}

impl SurroundComposite {
    pub(crate) fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("surround_uniform_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                // Read texel for texel, never filtered: R32Float is not
                // filterable everywhere.
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let read_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("surround_read_layout"),
            entries: &[texture(0), texture(1)],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("surround_composite_shader"),
            source: wgpu::ShaderSource::Wgsl(composite_source().into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("surround_composite_layout"),
            bind_group_layouts: &[Some(&uniform_layout), Some(&read_layout)],
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
        SurroundComposite { pipeline, uniform_layout, read_layout }
    }
}

/// A producer's three passes into [`SurroundTargets`]: its notes' cut summed
/// (`sum`) or kept strongest-wins (`max`), and their fill color under a depth
/// test (`color`). Built from the producer's own module, which has joined
/// surround.wgsl, with its own vertex stage and bind groups.
pub(crate) struct SurroundPipelines {
    sum: wgpu::RenderPipeline,
    max: wgpu::RenderPipeline,
    color: wgpu::RenderPipeline,
}

/// The entry points and vertex buffers a producer's [`SurroundPipelines`] are
/// built from.
pub(crate) struct SurroundStages<'a> {
    pub(crate) label: &'a str,
    pub(crate) vertex: &'a str,
    /// Writes [`surround_share`] of the note's cut into `r`.
    pub(crate) cut: &'a str,
    /// Writes the note's straight fill color, and `1 - cut` as depth.
    pub(crate) color: &'a str,
    pub(crate) buffers: &'a [wgpu::VertexBufferLayout<'a>],
}

impl SurroundPipelines {
    pub(crate) fn new(
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        layouts: &[Option<&wgpu::BindGroupLayout>],
        stages: SurroundStages<'_>,
    ) -> Self {
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(stages.label),
            bind_group_layouts: layouts,
            ..Default::default()
        });
        let pipeline = |fragment: &str, target: wgpu::ColorTargetState, depth| {
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
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(target)],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: depth,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let cut = |operation| {
            let component = wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation,
            };
            pipeline(
                stages.cut,
                wgpu::ColorTargetState {
                    format: CUT_FORMAT,
                    blend: Some(wgpu::BlendState { color: component, alpha: component }),
                    write_mask: wgpu::ColorWrites::RED,
                },
                None,
            )
        };
        SurroundPipelines {
            sum: cut(wgpu::BlendOperation::Add),
            max: cut(wgpu::BlendOperation::Max),
            color: pipeline(
                stages.color,
                wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                },
                Some(wgpu::DepthStencilState {
                    format: COLOR_DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
            ),
        }
    }
}

/// A producer's surround over its rect, one texel per device pixel: the cut,
/// the fill color and its depth, and the cut's uniform for the composite.
/// Held while the cut draws, and dropped with it or when the rect resizes,
/// which the producer keeps track of.
pub(crate) struct SurroundTargets {
    cut_view: wgpu::TextureView,
    /// Built the first time the cut fills and held from then on, since a fill
    /// dialled to 0 and back is one drag. A cut to black, the default, never
    /// pays for them.
    fill: Option<FillTargets>,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    /// The cut and the fill color as the composite reads them; the cut stands
    /// in the color's slot until there is one, since the composite reads that
    /// slot only while the cut fills.
    read_bind_group: wgpu::BindGroup,
    size: [u32; 2],
}

/// The strongest note's fill color over a producer's rect, and the depth that
/// picks it.
struct FillTargets {
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
}

/// One of a producer's targets over its rect, `size` device pixels across.
fn target(
    device: &wgpu::Device,
    label: &str,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
    size: [u32; 2],
) -> wgpu::TextureView {
    crate::gpu_memory::texture(
        device,
        &wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        },
    )
    .create_view(&Default::default())
}

/// What the composite reads: the cut, and the fill color or the cut again.
fn read_bind_group(
    device: &wgpu::Device,
    composite: &SurroundComposite,
    cut: &wgpu::TextureView,
    color: &wgpu::TextureView,
) -> wgpu::BindGroup {
    let view = |binding, view| wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    };
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("surround_read_bind_group"),
        layout: &composite.read_layout,
        entries: &[view(0, cut), view(1, color)],
    })
}

const READ: wgpu::TextureUsages =
    wgpu::TextureUsages::RENDER_ATTACHMENT.union(wgpu::TextureUsages::TEXTURE_BINDING);

impl SurroundTargets {
    pub(crate) fn new(
        device: &wgpu::Device,
        composite: &SurroundComposite,
        size: [u32; 2],
    ) -> Self {
        let cut_view = target(device, "surround_cut", CUT_FORMAT, READ, size);
        let uniform_buffer = crate::gpu_memory::buffer(
            device,
            &wgpu::BufferDescriptor {
                label: Some("surround_uniform"),
                size: std::mem::size_of::<SurroundUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );
        SurroundTargets {
            uniform_bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("surround_uniform_bind_group"),
                layout: &composite.uniform_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                }],
            }),
            read_bind_group: read_bind_group(device, composite, &cut_view, &cut_view),
            cut_view,
            fill: None,
            uniform_buffer,
            size,
        }
    }

    /// The notes' surround into these targets: the fill color while the cut
    /// fills, then the cut. `draw` binds the producer's own groups and vertex
    /// buffers and draws its notes; it is called once per pass, after the
    /// pipeline is set.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &SurroundPipelines,
        composite: &SurroundComposite,
        uniform: SurroundUniform,
        mut draw: impl FnMut(&mut wgpu::RenderPass<'_>),
    ) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
        if uniform.fills() {
            if self.fill.is_none() {
                let fill = FillTargets {
                    color_view: target(device, "surround_color", COLOR_FORMAT, READ, self.size),
                    depth_view: target(
                        device,
                        "surround_color_depth",
                        COLOR_DEPTH_FORMAT,
                        wgpu::TextureUsages::RENDER_ATTACHMENT,
                        self.size,
                    ),
                };
                self.read_bind_group =
                    read_bind_group(device, composite, &self.cut_view, &fill.color_view);
                self.fill = Some(fill);
            }
            let fill = self.fill.as_ref().expect("built above");
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("surround_color"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &fill.color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                // Cleared to 1, the depth of no cut at all, which no fragment
                // of no cut can beat.
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &fill.depth_view,
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
            pass.set_pipeline(&pipelines.color);
            draw(&mut pass);
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("surround_cut"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.cut_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(if uniform.blends() { &pipelines.sum } else { &pipelines.max });
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
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        pass.set_bind_group(1, &self.read_bind_group, &[]);
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

    #[test]
    fn composite_uniform_matches_the_bound_shader_layout() {
        crate::uniforms::layout::check_binding::<SurroundUniform>(&composite_source(), 0, 0);
    }

    /// A blend too soft for f32's normal range is the strongest-wins cut, and
    /// one just above it is the smooth minimum.
    #[test]
    fn a_blend_below_f32s_reach_is_the_strongest_wins_cut() {
        let kappa =
            |blend| SurroundCut { amount: 1.0, blend, ..SurroundCut::OFF }.uniform(1.0).cut.0[3];
        let edge = SURROUND_KAPPA_MIN / SURROUND_HALO_BLEND_MAX;
        assert_eq!(kappa(edge * 0.99), 0.0);
        assert!(kappa(edge * 1.01) >= SURROUND_KAPPA_MIN);
        assert!((-1.0 / SURROUND_KAPPA_MIN).exp() >= f32::MIN_POSITIVE);
    }
}
