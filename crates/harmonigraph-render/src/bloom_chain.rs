//! The bloom post-process shared by the lattice, the roll and the spiral's glow.

use super::*;

/// The bloom chain's two working sizes for a picture `screen` pixels across:
/// the quarter, each side rounded up, and the half as exactly twice it.
///
/// Twice the quarter rather than half the screen rounded up, so the
/// downsample reads the half on an exact 2:1 grid at every size. Rounded
/// separately, a side 1 or 2 mod 4 left an odd half under a quarter more than
/// half of it, the tent slid off its texels across the pane, and a two-pixel
/// note's halo pulsed by 66% of its peak as it crossed them (98% under the box
/// before it). The half's texels come out a hair under two screen pixels, which
/// every stage already maps through: the roll's notes by the per-axis scale
/// `RollUniforms::drawn_into` measures, everything else by uv.
pub(crate) fn bloom_sizes(screen: [u32; 2]) -> ([u32; 2], [u32; 2]) {
    let quarter = screen.map(|n| n.div_ceil(4).max(1));
    (quarter.map(|q| 2 * q), quarter)
}

/// The bloom post-process's targets and bind groups: a soft-knee threshold
/// into half the picture's SCREEN size, a tent-filtered downsample to a
/// quarter, then a separable blur ping-ponging between two quarter-res
/// textures, read back up through a B-spline (`bloom_sample` in `blit.wgsl`).
/// The two filters at the grid changes are what keep a halo's shape still as
/// its source slides across the quarter grid.
///
/// One chain, every picture. The lattice feeds it the scene without its
/// labels; the piano roll feeds it the notes rendered again offscreen
/// (`crate::roll`); the spiral's dots feed it through `crate::glow`. That they
/// are the same four steps in the same order over the same fractions is the
/// whole of what makes one bloom strength mean one halo, and it is a claim a
/// second copy cannot keep: the step that matters
/// most is WHERE the threshold sits, and a chain that thresholds after the
/// downsample instead of before it measures a thin shape that has already been
/// averaged twice, so a ribbon gets a fraction of the halo the node it lit up
/// gets from the identical color.
///
/// The blurs run at a quarter, which is what makes the halo wide and cheap;
/// the threshold runs at a half, which is what makes it measure the picture
/// rather than a smear of it.
pub(super) struct BloomChain {
    scale_buffer: wgpu::Buffer,
    scale_bind_group: wgpu::BindGroup,
    screen_size: [u32; 2],
    /// The descriptor format shared by all three bloom targets.
    #[cfg(test)]
    pub(super) format: wgpu::TextureFormat,
    /// The thresholded picture at half the screen size.
    pub(super) half_view: wgpu::TextureView,
    /// The blur's ping-pong pair, and A is where the chain ENDS — whatever
    /// composites the halo samples A, so the vertical blur must land there.
    pub(super) quarter_a_view: wgpu::TextureView,
    pub(super) quarter_b_view: wgpu::TextureView,
    /// Bind groups, named by the pass that USES them (source texture + the
    /// shared sampler): bright samples the caller's picture, downsample the
    /// half, blur_h quarter A, blur_v quarter B.
    bright_bind_group: wgpu::BindGroup,
    downsample_bind_group: wgpu::BindGroup,
    blur_h_bind_group: wgpu::BindGroup,
    blur_v_bind_group: wgpu::BindGroup,
}

/// The four pipelines [`BloomChain::run`] steps through, in that order.
///
/// Passed in rather than held: they are built per target format, and each
/// caller has its own (the lattice writes an offscreen texture, the roll and
/// the glow the surface egui handed them).
pub(super) struct BloomPipelines<'a> {
    pub(super) bright: &'a wgpu::RenderPipeline,
    pub(super) downsample: &'a wgpu::RenderPipeline,
    pub(super) blur_h: &'a wgpu::RenderPipeline,
    pub(super) blur_v: &'a wgpu::RenderPipeline,
}

impl BloomChain {
    pub(super) fn scale_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom_scale_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(16),
                },
                count: None,
            }],
        })
    }

    /// The blur's scale per axis, in units of its own nine-tap profile, and
    /// then the half target's size, which the bright pass needs to know how
    /// much of the picture each of its texels covers.
    ///
    /// Less the spread the tent downsample (1/8 of a quarter texel squared
    /// over the box it replaced) and the B-spline read (1/6 over bilinear) add
    /// on their own, against the profile's variance of 2.854 taps squared.
    /// Those are fixed in quarter texels, so without this they widened a small
    /// pane's halo more than a large one's at the same width in points:
    /// `bloom_width_follows_resolution` read 19% at half a pixel per point,
    /// against 15% allowed. The box and bilinear spread the chain had before
    /// is not given back here, as it never was. At most half the blur is given
    /// up, which a pane small enough to need more draws a halo narrower than a
    /// quarter texel anyway.
    fn scales(screen: [u32; 2], owner_pixels: egui::Vec2) -> [f32; 4] {
        const FIXED_SPREAD: f32 = (1.0 / 8.0 + 1.0 / 6.0) / 2.854;
        let radius = owner_pixels.min_elem().max(1.0) / 1080.0;
        let axis = |n: u32| {
            let scale = radius * (n.div_ceil(4).max(1) * 4) as f32 / n.max(1) as f32;
            (scale * scale - FIXED_SPREAD).max(0.25 * scale * scale).sqrt()
        };
        let ([hw, hh], _) = bloom_sizes(screen);
        [axis(screen[0]), axis(screen[1]), hw as f32, hh as f32]
    }

    pub(super) fn set_owner_size(&self, queue: &wgpu::Queue, owner_pixels: egui::Vec2) {
        queue.write_buffer(
            &self.scale_buffer,
            0,
            bytemuck::cast_slice(&Self::scales(self.screen_size, owner_pixels)),
        );
    }

    /// Build the chain over `source`, at fractions of `screen_size` device
    /// pixels.
    ///
    /// `screen_size` is the picture's size ON SCREEN and not the size of
    /// `source`: the lattice's scene texture is render-scaled and the roll's
    /// note texture is already halved, and in both cases what the halo's width
    /// must be a constant share of is the screen.
    pub(super) fn new(
        device: &wgpu::Device,
        label: &str,
        format: wgpu::TextureFormat,
        filter_layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        source: &wgpu::TextureView,
        screen_size: [u32; 2],
    ) -> Self {
        let tex = |label: String, w: u32, h: u32| {
            crate::gpu_memory::texture(
                device,
                &wgpu::TextureDescriptor {
                    label: Some(&label),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            )
            .create_view(&Default::default())
        };
        let ([hw, hh], [qw, qh]) = bloom_sizes(screen_size);
        let half_view = tex(format!("{label}_bloom_half"), hw, hh);
        let quarter_a_view = tex(format!("{label}_bloom_quarter_a"), qw, qh);
        let quarter_b_view = tex(format!("{label}_bloom_quarter_b"), qw, qh);
        let filter_bg = |label: String, source: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&label),
                layout: filter_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            })
        };

        let scale_buffer = crate::gpu_memory::buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("bloom_scale"),
                contents: bytemuck::cast_slice(&Self::scales(
                    screen_size,
                    egui::vec2(screen_size[0] as f32, screen_size[1] as f32),
                )),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            },
        );
        let scale_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom_scale_binding"),
            layout: &Self::scale_layout(device),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: scale_buffer.as_entire_binding(),
            }],
        });
        BloomChain {
            scale_buffer,
            scale_bind_group,
            screen_size,
            #[cfg(test)]
            format,
            bright_bind_group: filter_bg(format!("{label}_bright_bind_group"), source),
            downsample_bind_group: filter_bg(format!("{label}_downsample_bind_group"), &half_view),
            blur_h_bind_group: filter_bg(format!("{label}_blur_h_bind_group"), &quarter_a_view),
            blur_v_bind_group: filter_bg(format!("{label}_blur_v_bind_group"), &quarter_b_view),
            half_view,
            quarter_a_view,
            quarter_b_view,
        }
    }

    /// The four full-screen passes, in the one order they may run in:
    /// bright-pass into half res, downsample to quarter, then a separable blur
    /// ping-ponging quarter A -> B (horizontal) -> A (vertical). Whatever
    /// composites the halo samples quarter A, so the vertical blur MUST be the
    /// step that lands there.
    ///
    /// This is the only place that ordering is written down; the pipelines
    /// themselves are built by each caller.
    pub(super) fn run(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: BloomPipelines<'_>,
        label: &str,
        end_stamp: Option<&wgpu::QuerySet>,
    ) {
        let steps: [BloomStep; 4] = [
            (pipelines.bright, &self.bright_bind_group, &self.half_view),
            (pipelines.downsample, &self.downsample_bind_group, &self.quarter_a_view),
            (pipelines.blur_h, &self.blur_h_bind_group, &self.quarter_b_view),
            (pipelines.blur_v, &self.blur_v_bind_group, &self.quarter_a_view),
        ];
        for (index, (pipeline, bind_group, target)) in steps.into_iter().enumerate() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(&format!("{label}_bloom_pass")),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: end_stamp.filter(|_| index == 3).map(|query_set| {
                    wgpu::RenderPassTimestampWrites {
                        query_set,
                        beginning_of_pass_write_index: None,
                        end_of_pass_write_index: Some(1),
                    }
                }),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            // Everything but the downsample reads it. The roll's
            // `fs_bright_coverage` does not, and a group its layout lacks is
            // simply not read.
            if index != 1 {
                pass.set_bind_group(1, &self.scale_bind_group, &[]);
            }
            pass.draw(0..4, 0..1);
        }
    }
}
