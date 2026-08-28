//! The distance a lattice name's Shadow is cast from: the ink mask every name
//! on a pane is drawn into, and the jump flood that turns it into a nearest-ink
//! coordinate at every pixel.
//!
//! field.wgsl holds the arithmetic and the argument for it. What is here is the
//! plumbing that argument needs: three pane-sized textures, a ping-pong pair of
//! bind groups over two of them, and the schedule of halving steps the chain
//! runs. Both readers are in text.wgsl, which is where a Shadow's own curve
//! lives.

use crate::{text, wgpu};

const FIELD_SRC: &str = include_str!("shaders/field.wgsl");

/// The largest jump the chain will start from, as a power of two in pixels.
///
/// A bound on the PASS COUNT rather than on the Shadow: 2^14 is 16384 px, past
/// the widest pane any device here reports, so the cap is unreachable in the
/// picture and is here to keep a nonsense uniform — a NaN node radius, an
/// infinity out of a corrupt blob — from asking for an unbounded chain.
const MAX_LOG_STEP: u32 = 14;

/// `SHADOW_STOP` in text.wgsl and lattice.wgsl, which is what makes this a
/// BOUND rather than a fourth copy of the standoff's curve.
///
/// `shadow_stop` is `inner + SHADOW_STOP * (edge - inner)` over an `inner`
/// somewhere in `[0, edge]`, so it is at most `SHADOW_STOP * edge` whatever the
/// fade is set to — and a flood run further than the coverage reaches costs
/// passes rather than correctness. Reading the exact stop here would mean
/// spelling the fade, the shape and their two floors in Rust as well, and
/// `the_names_shadow_is_the_rings_own_curve` exists to keep that number in one
/// place.
const SHADOW_STOP: f32 = 2.0;

/// How far the flood has to carry a seed, in pixels, for a pane whose Shadow is
/// `shadow` node radii wide over a node of `node_points` points.
///
/// Zero where the bar is at its bottom: no name casts anything, and the caller
/// skips the chain entirely rather than running it over an empty mask.
/// `shadow_edge`'s own floor is carried so this stays an upper bound at every
/// width: a Shadow dialled to almost nothing still has an edge of a thousandth
/// of a point, and a bound that forgot it would be under the stop it bounds.
/// Written as the POSITIVE test so a NaN out of a corrupt blob falls to the
/// zero rather than through it — every comparison against a NaN is false, and
/// the negated form is the one that lets one past.
pub(crate) fn reach_px(shadow: f32, node_points: f32, pixels_per_point: f32) -> f32 {
    if shadow > 0.0 && node_points > 0.0 {
        return SHADOW_STOP * (shadow * node_points).max(0.001) * pixels_per_point;
    }
    0.0
}

/// The jumps the chain takes, largest first, halving to 1.
///
/// A jump flood carries a seed `2^k + 2^(k-1) + ... + 1` pixels, one short of
/// `2^(k+1)`, so starting at the first power of two at or above `reach` reaches
/// every seed within it with the whole of the top jump to spare.
///
/// The tail is doubled when the count comes out ODD, which buys two things at
/// the price of one pass. It is the standard extra step-of-1 refinement — the
/// flood is exact for a single seed and approximate for many, and the errors it
/// makes are all within a pixel or two of a territory boundary, which one more
/// local pass mops up. And it lands the answer in a FIXED one of the two
/// textures, so the readers bind one bind group rather than choosing by parity.
/// The POSITIVE test again, and for [`reach_px`]'s reason: a NaN reach takes
/// the empty schedule rather than `log2`'s answer for one.
pub(crate) fn steps(reach: f32) -> Vec<i32> {
    if reach >= 1.0 {
        let top = (reach.log2().ceil() as u32).min(MAX_LOG_STEP);
        let mut out: Vec<i32> = (0..=top).rev().map(|k| 1i32 << k).collect();
        if out.len() % 2 == 1 {
            out.push(1);
        }
        return out;
    }
    // A Shadow reaching under a pixel has nothing to carry: the only fragments
    // it says anything about are the inked ones, and the seed pass leaves each
    // of those holding ITSELF. What a letter's antialiased edge is owed there
    // comes off the mask directly (`field_standoff` in text.wgsl), not out of
    // the flood.
    //
    // Empty is even, which is the property the readers depend on.
    Vec::new()
}

/// One pane's field: the ink mask, the flood's ping-pong pair, and the bind
/// groups over them.
///
/// Pane-sized and held on the pane's own [`Offscreen`](crate::Offscreen), which
/// is what keeps a lattice with two panes from sharing one — the mask is every
/// name on ONE pane, and a field shared between two would cast each pane's
/// shadows from the other's letters.
pub(crate) struct FieldTarget {
    /// The textures behind two of the views below, kept only so a test can put
    /// a synthetic mask in and read the flood's answer back out
    /// (`the_flood_answers_the_true_distance_between_two_strokes`). A view
    /// holds its own texture alive, so nothing here depends on them.
    ///
    /// A field that cannot be measured directly is one whose errors can only be
    /// found by looking at a picture, and the artefact this replaces went
    /// unnoticed for exactly that reason. The `COPY_DST`/`COPY_SRC` usages the
    /// pair needs are granted unconditionally, being a property of the texture
    /// rather than of the build.
    #[cfg(test)]
    ink: wgpu::Texture,
    #[cfg(test)]
    fields: [wgpu::Texture; 2],
    /// Every name's coverage and strength, unioned (`fs_glyph_ink`).
    pub(crate) ink_view: wgpu::TextureView,
    /// The flood's two targets. `steps` always runs an even number of passes,
    /// so the finished field is in `views[0]` and `reader` names it.
    views: [wgpu::TextureView; 2],
    /// Reading `views[i]`, for the pass that writes `views[i ^ 1]`.
    chain: [wgpu::BindGroup; 2],
    /// The finished field and the mask, as both readers take them
    /// (`text::field_bind_group_layout`).
    pub(crate) reader: wgpu::BindGroup,
    /// One `Jump` per step, at dynamic-offset stride.
    ///
    /// Rewritten every frame the chain runs rather than cached against the last
    /// schedule. It is at most seventeen aligned cells — four kilobytes against
    /// the three pane-sized textures beside it — and holding a cache would mean
    /// holding it MUTABLY, which is the one thing the encode path cannot do
    /// while it is also reading this frame's pipelines off the same resources.
    jumps: wgpu::Buffer,
}

/// What one step's uniform holds: the jump, in `x`.
///
/// A whole `vec4<i32>` because that is what the uniform address space lays a
/// struct out on — `Jump` in field.wgsl, which has to be the same 16 bytes or
/// the pipeline is rejected at first paint for a `min_binding_size` it cannot
/// meet. Written [`JUMP_STRIDE`] apart, which is the alignment the BINDING
/// wants and a different number entirely.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Jump {
    step: [i32; 4],
}

/// The stride the jump buffer's dynamic offsets step by:
/// `min_uniform_buffer_offset_alignment`'s own guaranteed value, which every
/// backend wgpu targets meets. Sixteen bytes of `Jump` in each.
const JUMP_STRIDE: u64 = 256;

impl FieldTarget {
    pub(crate) fn new(
        device: &wgpu::Device,
        chain_layout: &wgpu::BindGroupLayout,
        reader_layout: &wgpu::BindGroupLayout,
        size: [u32; 2],
    ) -> Self {
        let texture = |label, format, extra| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size[0].max(1),
                    height: size[1].max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | extra,
                view_formats: &[],
            })
        };
        let ink = texture("lattice_name_ink", text::INK_FORMAT, wgpu::TextureUsages::COPY_DST);
        let fields = [
            texture("lattice_name_field_0", text::FIELD_FORMAT, wgpu::TextureUsages::COPY_SRC),
            texture("lattice_name_field_1", text::FIELD_FORMAT, wgpu::TextureUsages::COPY_SRC),
        ];
        let ink_view = ink.create_view(&Default::default());
        let views = [
            fields[0].create_view(&Default::default()),
            fields[1].create_view(&Default::default()),
        ];
        let jumps = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lattice_name_field_jumps"),
            size: JUMP_STRIDE * (MAX_LOG_STEP as u64 + 3),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let chain_group = |src: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("lattice_name_field_chain"),
                layout: chain_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(src),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &jumps,
                            offset: 0,
                            size: std::num::NonZeroU64::new(std::mem::size_of::<Jump>() as u64),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&ink_view),
                    },
                ],
            })
        };
        let chain = [chain_group(&views[0]), chain_group(&views[1])];
        let reader = text::field_bind_group(device, reader_layout, &views[0], &ink_view);
        FieldTarget {
            #[cfg(test)]
            ink,
            #[cfg(test)]
            fields,
            ink_view,
            views,
            chain,
            reader,
            jumps,
        }
    }

    /// Run the chain for a Shadow reaching `reach` pixels, leaving the finished
    /// field in `views[0]`.
    ///
    /// The seed pass binds `chain[1]` and reads nothing from it: the seed comes
    /// off the ink mask, which every group in the pair carries, and the `src`
    /// slot it does not read has to name SOME texture for the layout to be
    /// satisfied. Naming the one it is about to write would be a pass sampling
    /// its own attachment.
    pub(crate) fn run(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: (&wgpu::RenderPipeline, &wgpu::RenderPipeline),
        reach: f32,
    ) {
        let (seed_pipeline, step_pipeline) = pipelines;
        let schedule = steps(reach);
        let jumps: Vec<u8> = schedule
            .iter()
            .flat_map(|&step| {
                let mut cell = [0u8; JUMP_STRIDE as usize];
                let jump = Jump { step: [step, 0, 0, 0] };
                cell[..std::mem::size_of::<Jump>()].copy_from_slice(bytemuck::bytes_of(&jump));
                cell
            })
            .collect();
        if !jumps.is_empty() {
            queue.write_buffer(&self.jumps, 0, &jumps);
        }
        let mut pass = |target: &wgpu::TextureView,
                        pipeline: &wgpu::RenderPipeline,
                        group: &wgpu::BindGroup,
                        offset: u32| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("lattice_name_field_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    // Every texel is written, so the load is discarded work
                    // whatever it says. Cleared rather than `Load` because a
                    // clear is the cheaper of the two on a tiler and neither
                    // survives the draw.
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
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[offset]);
            pass.draw(0..4, 0..1);
        };
        pass(&self.views[0], seed_pipeline, &self.chain[1], 0);
        let mut src = 0usize;
        for i in 0..schedule.len() {
            pass(
                &self.views[src ^ 1],
                step_pipeline,
                &self.chain[src],
                (i as u64 * JUMP_STRIDE) as u32,
            );
            src ^= 1;
        }
        debug_assert_eq!(src, 0, "the schedule is even, so the field lands in views[0]");
    }
}

/// The bindings a flood pass takes: the field it reads, the jump it takes, and
/// the ink mask the seed pass starts from.
///
/// One layout for both passes though each reads about half of it — see the
/// comment on the group in field.wgsl.
pub(crate) fn chain_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("lattice_name_field_layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: std::num::NonZeroU64::new(std::mem::size_of::<Jump>() as u64),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    })
}

/// The chain's two pipelines: the seed off the ink mask, then the step that
/// runs over its own output.
///
/// Both write [`text::FIELD_FORMAT`] with no blending at all — a flood pass
/// decides each texel outright, and a blend is a way of mixing two answers
/// where here the later one is simply the better one.
pub(crate) fn create_pipelines(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("field_shader"),
        source: wgpu::ShaderSource::Wgsl(FIELD_SRC.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("lattice_name_field_pipeline_layout"),
        bind_group_layouts: &[Some(layout)],
        ..Default::default()
    });
    let pipeline = |entry: &str| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(entry),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_field"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: text::FIELD_FORMAT,
                    blend: None,
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
        })
    };
    (pipeline("fs_field_seed"), pipeline("fs_field_step"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The schedule reaches at least as far as it is asked to, and lands on an
    /// even number of passes so the readers can name one texture.
    ///
    /// The reach is what a jump flood actually carries — the SUM of its jumps —
    /// rather than the first of them, which is the number it is easy to check
    /// and the wrong one: a chain starting at 32 with no tail carries 32, and
    /// the same chain down to 1 carries 63.
    #[test]
    fn a_floods_jumps_reach_past_the_shadow_they_are_built_for() {
        for reach in [1.0f32, 2.0, 3.0, 31.0, 32.0, 33.0, 200.0, 4000.0] {
            let schedule = steps(reach);
            let carried: i32 = schedule.iter().sum();
            assert!(
                carried as f32 >= reach,
                "a chain {schedule:?} carries {carried} and is asked for {reach}",
            );
            assert_eq!(schedule.len() % 2, 0, "an odd chain lands the field in views[1]");
            assert_eq!(*schedule.last().expect("a step"), 1, "the tail is a local pass");
        }
    }

    /// A Shadow reaching under a pixel runs no flood at all — the seed pass has
    /// already answered every fragment it can speak for.
    ///
    /// Which is where the Shadow bar's own bottom lands: the width is 0, the
    /// coverage is the ink's own and nothing else, and the chain would carry it
    /// nowhere at the price of a full-pane pass per step.
    #[test]
    fn a_shadow_under_a_pixel_runs_no_flood() {
        for reach in [0.0f32, 0.5, 0.999] {
            assert!(steps(reach).is_empty(), "a reach of {reach} asked for a flood");
        }
    }

    /// A reach past every pane still terminates, and does so at the cap rather
    /// than by looping on a number no `log2` can answer for.
    #[test]
    fn a_nonsense_reach_stops_at_the_cap() {
        for reach in [f32::INFINITY, f32::NAN, 1.0e30, -5.0] {
            let schedule = steps(reach);
            assert!(
                schedule.len() <= MAX_LOG_STEP as usize + 2,
                "a reach of {reach} asks for {} passes",
                schedule.len(),
            );
            assert_eq!(schedule.len() % 2, 0, "an odd chain lands the field in views[1]");
        }
    }

    /// The two shaders agree on what a seed IS: the sentinel that stands for
    /// none within reach, and the coverage that makes a texel one.
    ///
    /// Both have to be written twice — there is no linkage between shader
    /// modules here — and both fail SILENTLY if they part. A sentinel the
    /// reader does not recognise makes every pixel a seed at (65535, 65535),
    /// and every name's shadow a smooth gradient toward the bottom-right corner
    /// of the pane. A floor that has parted puts the reader's contour somewhere
    /// inside the texels the flood seeds rather than on the edge of them, which
    /// draws every name's standoff a fraction of a texel off its own letters.
    #[test]
    fn a_names_field_and_its_flood_agree_on_no_seed() {
        let value = |src: &str, name: &str| {
            src.lines()
                .find(|l| l.trim_start().starts_with(&format!("const {name}")))
                .and_then(|l| l.split('=').nth(1))
                .map(|v| v.trim().trim_end_matches(';').trim_end_matches('u').to_string())
                .unwrap_or_else(|| panic!("both shaders declare {name}"))
        };
        for name in ["NO_SEED", "INK_FLOOR"] {
            assert_eq!(
                value(FIELD_SRC, name),
                value(crate::text::TEXT_SRC, name),
                "the flood's {name} and the reader's have drifted apart",
            );
        }
    }

    /// The flood answers the TRUE distance, everywhere inside its reach, for a
    /// pair of strokes as far apart as two letters of a name.
    ///
    /// This is the measurement the artefact it replaces could not pass. A
    /// dilation sampled at N offsets is, wherever the Shadow's coverage is
    /// flat, a binary dilation by a disc; N taps on a spiral sit about
    /// `R / sqrt(N)` apart, so at a reach of 32 px, 48 of them leave gaps four
    /// pixels wide between the copies of a two-pixel stroke, and the shortfall
    /// draws the letters again inside their own shadow. What is asserted is
    /// what makes that impossible: an exact distance at every pixel, at a cost
    /// that does not move with the reach.
    ///
    /// A pixel of tolerance, because the flood answers in whole texels — a seed
    /// is a texel centre, so the nearest one is right to within the grid it
    /// stands on and no finer.
    #[test]
    fn the_flood_answers_the_true_distance_between_two_strokes() {
        const SIZE: [u32; 2] = [256, 128];
        const STROKES: [u32; 4] = [100, 101, 140, 141];
        const REACH: f32 = 32.0;

        let Some((device, queue)) = crate::gpu_harness::headless_device() else {
            return;
        };
        let chain_layout = chain_bind_group_layout(&device);
        let reader_layout = text::field_bind_group_layout(&device);
        let target = FieldTarget::new(&device, &chain_layout, &reader_layout, SIZE);
        let pipelines = create_pipelines(&device, &chain_layout);

        // Two solid strokes two pixels wide, forty apart: a name's two stems,
        // and the gap the sampled dilation could not fill.
        let mut ink = vec![0u8; (SIZE[0] * SIZE[1] * 2) as usize];
        for y in 0..SIZE[1] {
            for x in STROKES {
                let at = ((y * SIZE[0] + x) * 2) as usize;
                ink[at] = 255;
                ink[at + 1] = 255;
            }
        }
        queue.write_texture(
            target.ink.as_image_copy(),
            &ink,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE[0] * 2),
                rows_per_image: Some(SIZE[1]),
            },
            wgpu::Extent3d { width: SIZE[0], height: SIZE[1], depth_or_array_layers: 1 },
        );

        let mut encoder = device.create_command_encoder(&Default::default());
        target.run(&queue, &mut encoder, (&pipelines.0, &pipelines.1), REACH);
        queue.submit([encoder.finish()]);
        let bytes = crate::gpu_harness::readback(&device, &queue, &target.fields[0], SIZE);

        // The row through the middle, which every stroke crosses: the field is
        // the same at every row here, so one is the whole reading.
        let y = SIZE[1] / 2;
        let mut checked = 0;
        for x in 0..SIZE[0] {
            let at = ((y * SIZE[0] + x) * 4) as usize;
            let seed = [
                u16::from_le_bytes([bytes[at], bytes[at + 1]]),
                u16::from_le_bytes([bytes[at + 2], bytes[at + 3]]),
            ];
            let truth = STROKES.iter().map(|&s| x.abs_diff(s)).min().expect("four strokes");
            if truth as f32 > REACH {
                continue;
            }
            assert_ne!(
                seed[0], 65535,
                "no seed at x={x}, {truth} px from a stroke inside the reach"
            );
            let found = (seed[0] as f32 - x as f32).hypot(seed[1] as f32 - y as f32);
            assert!(
                (found - truth as f32).abs() <= 1.0,
                "at x={x} the flood answers {found:.2} px where the nearest stroke is {truth}",
            );
            checked += 1;
        }
        // The loop is the assertion, so a loop that ran over nothing is a
        // green test measuring an empty field.
        assert!(checked > 100, "only {checked} pixels stood inside the reach");
    }

    /// The bound the chain is sized by is the shader's own `SHADOW_STOP`, which
    /// is what makes `reach_px` a bound rather than a fourth copy of the
    /// standoff's curve.
    #[test]
    fn the_floods_reach_is_bounded_by_the_shaders_own_stop() {
        let shader = crate::text::TEXT_SRC;
        let line = shader
            .lines()
            .find(|l| l.trim_start().starts_with("const SHADOW_STOP"))
            .expect("text.wgsl declares SHADOW_STOP");
        let value: f32 = line
            .split('=')
            .nth(1)
            .and_then(|v| v.trim().trim_end_matches(';').parse().ok())
            .expect("a number");
        assert_eq!(value, SHADOW_STOP, "the flood's bound and the curve's stop have parted");
    }
}
