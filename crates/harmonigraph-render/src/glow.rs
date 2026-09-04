//! A halo over marks a pane has already drawn for itself: the lattice's own
//! bloom chain, run over a copy of those marks this crate re-renders, and laid
//! back over the finished picture as light.
//!
//! **What it does not do.** It draws nothing sharp. The lattice and the piano
//! roll both render their own picture through a callback and grow the halo out
//! of the same pass; this one is handed marks a pane has painted through egui's
//! tessellator and adds only what egui cannot draw at all. Which is the whole
//! shape of the thing: `paint` sets one pipeline (`fs_bloom_add`) and draws one
//! quad.
//!
//! **Why the pane keeps its own marks.** The roll moved its notes onto the GPU
//! because their geometry was the frame's dominant upload cost — six figures of
//! vertices re-sent every frame. A handful of dots is not that: the spiral
//! paints one per sounding voice, and moving them here would buy nothing while
//! costing the pane the egui shapes its own geometry tests read.
//!
//! **Why the copy is re-rendered rather than read back.** [`crate::roll`]'s
//! argument, and it holds harder here: the sharp layer stays exactly where egui
//! put it, and a pass that read the finished pane back would have to isolate
//! the marks from everything drawn under them — on the spiral, a spectrum that
//! fills the disc — which is not something a pixel can be asked.
//!
//! **A chain per live copy, keyed on `pane_id`.** egui-wgpu runs every
//! `prepare` before any `paint`, and the chain is sized in device pixels off
//! the rect it covers — so two copies sharing one chain at different sizes tear
//! it down and rebuild it twice within a frame, and the first then stretches
//! the second's quarter A, laid out in the second's local coordinates, across
//! its own rect. [`crate::roll`] spends a pane map on exactly that, and this
//! one is the same map for the same reason: a hand-written offline layout can
//! name `Spiral` twice at unequal rects, which is the arrangement that reaches
//! it (`harmonigraph_ui::draw_pane` hands each placement its index).
//!
//! **A copy is evicted when it stops preparing**, on the clock of `prepare`
//! calls [`crate::roll`] keeps for the same purpose: there is no teardown to
//! hang the drop on, so the copies still preparing are the only evidence of
//! which ones exist, and the sweep runs from whichever one is. What it reclaims
//! is a chain's worth of textures per retired copy — the half-size copy, the
//! thresholded half and the two quarters, about 2.7 MB for a 600x450-point tab
//! on a Retina display.
//!
//! The LAST copy is the one it cannot reclaim: the spiral is the only caller
//! here, so a hidden Spiral tab leaves nothing preparing to sweep with and its
//! chain stands until something asks for a halo again. That holds one chain,
//! not one per tab ever opened, which is what makes the sweep worth its map.

use std::collections::HashMap;

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};

use crate::{create_vertex_buffer, wgpu, EGUI_BLEND};

const GLOW_SRC: &str = include_str!("shaders/glow.wgsl");

/// Entry points the glow shader must provide: the vertex stage, and each of the
/// two shadings [`create_disc_pipeline`] picks between. Its entry point is
/// assembled from the shading's name, so a rename in the WGSL is a panic at
/// pipeline creation and nothing sooner.
#[cfg(test)]
const GLOW_ENTRY_POINTS: &[&str] = &["vs_disc", "fs_disc_gamma", "fs_disc_linear"];

/// One mark to grow a halo out of: a filled disc in screen POINTS, exactly
/// where and as big as the pane painted it, in the color it painted it.
///
/// A disc and only a disc. What feeds a bloom is the shape's light rather than
/// its outline — the chain thresholds this at half the pane's size and blurs it
/// at a quarter — so a mark's kind matters much less here than its ink does,
/// and a caller with a different shape is better served by widening this than
/// by a second callback growing a second halo out of the same strength.
///
/// A DARK companion mark is not one of these: the spiral backs each dot with a
/// black disc and the roll wraps each note in a black outline, and black is the
/// one thing that cannot bloom. Drawn here it would only take light out of the
/// halo the colored mark does grow.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GlowDot {
    /// Centre, in screen points.
    pub center: [f32; 2],
    /// Radius, in points — the mark's painted extent, filled to its edge.
    pub radius: f32,
    /// Premultiplied sRGB bytes, straight out of [`egui::Color32`].
    pub color: [u8; 4],
}

impl GlowDot {
    pub(crate) const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<GlowDot>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x2, // center
            1 => Float32,   // radius
            2 => Unorm8x4,  // color
        ],
    };
}

/// Lay a halo over `rect`, grown from `dots`.
///
/// `rect` is what the chain covers, so it wants to be the region the marks can
/// reach and no more — resolution spent outside it is resolution the halo does
/// not get. Add the callback to the painter AFTER the marks it belongs to, and
/// before anything that must stay out of the light (a label, most of all: see
/// `crate::text`).
///
/// `strength` is the lattice's own bloom strength, through
/// [`crate::bloom_strength`] for the reason that function exists. 0 skips the
/// whole thing — no chain is built and no pass runs.
///
/// `pane_id` must be unique per halo grown in the same frame, and the same
/// across frames for one live copy of a pane: each id keeps a chain of its own,
/// so an id minted per frame would build a chain per frame and hold every one
/// of them until the sweep aged it out.
pub fn glow_paint_callback(
    rect: egui::Rect,
    dots: Vec<GlowDot>,
    strength: f32,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
) -> egui::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(
        rect,
        GlowCallback { rect, dots, strength, target_format, pane_id },
    )
}

/// Per-frame draw data, built on the UI thread.
struct GlowCallback {
    /// The region the halo covers in points. `paint` is handed this as the
    /// callback's viewport; `prepare` is handed nothing, and needs it to size
    /// the chain, so it rides here too.
    rect: egui::Rect,
    dots: Vec<GlowDot>,
    strength: f32,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
}

/// The disc pass's own uniforms (`Locals` in glow.wgsl).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GlowUniforms {
    /// Where the target starts in points (xy) and how big it is (zw).
    viewport: [f32; 4],
    /// The antialiasing ramp in x; the rest is the 16 bytes a uniform block
    /// takes.
    feather: [f32; 4],
}

/// The bloom's strength, alone in its own buffer (`AddUniforms` in blit.wgsl).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct StrengthUniforms {
    /// Strength in x; the rest is the 16 bytes a uniform block takes.
    strength: [f32; 4],
}

/// GPU objects cached across frames in egui-wgpu's `CallbackResources`.
struct GlowResources {
    /// The marks again, offscreen — the only geometry this callback owns.
    disc_pipeline: wgpu::RenderPipeline,
    locals_layout: wgpu::BindGroupLayout,
    /// The chain's four post passes and the one that lays the result over the
    /// pane. The four are the lattice's, out of the same blit.wgsl and stepped
    /// through by the same [`crate::BloomChain::run`], so the halo a mark grows
    /// here is the halo a node grows there: same threshold at the same
    /// resolution, same knee, same kernel, same fractions of the pane's screen
    /// size. Only the target FORMAT is this caller's own, which is why the
    /// pipelines are its own and the chain is not.
    bright_pipeline: wgpu::RenderPipeline,
    downsample_pipeline: wgpu::RenderPipeline,
    blur_h_pipeline: wgpu::RenderPipeline,
    blur_v_pipeline: wgpu::RenderPipeline,
    add_pipeline: wgpu::RenderPipeline,
    /// One sampled texture + the shared sampler (the three chain passes).
    filter_layout: wgpu::BindGroupLayout,
    /// The same, plus the strength (the pass into the egui pass).
    add_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    target_format: wgpu::TextureFormat,
    /// One chain per live copy of a pane. A copy appears on the first frame
    /// that asks it for a halo — a strength of 0 pays for none of it — and
    /// leaves on [`GlowPane::evict_unseen`].
    panes: HashMap<u64, GlowPane>,
    /// Counts `prepare` calls, which is what [`GlowPane::last_seen`] is stamped
    /// with — a clock the callback already has, where a frame count would need
    /// one to be plumbed in.
    prepares: u64,
}

/// How many `prepare` calls a copy may go unseen before its chain is dropped.
///
/// A copy prepares once per frame while it is on screen and this clock counts
/// every copy's prepare, so it is two seconds at 60 fps with one halo on screen
/// and proportionally less with more. Long enough that a pane hidden for a
/// frame keeps its textures, short enough that a closed one is not still
/// holding four of them a minute later.
const PANE_TTL_PREPARES: u64 = 120;

/// The picture to bloom and the lattice's own [`crate::BloomChain`] over it:
/// the marks rendered again offscreen, thresholded, blurred separably, and
/// added back over the pane as light.
struct GlowPane {
    /// The marks again, at HALF the pane's screen size — the picture the
    /// chain's threshold reads, standing where the lattice's own scene texture
    /// stands.
    ///
    /// Half rather than full is the one place this picture and the lattice's
    /// are not the same thing, and [`crate::roll`]'s note texture makes the
    /// same trade for the same reason: `fs_bright` only ever SAMPLES at half,
    /// so drawing there skips a resample, at the cost of measuring a mark
    /// narrower than one of these pixels by the disc shader's own box filter
    /// rather than by a bilinear tap over a sharper raster.
    discs_view: wgpu::TextureView,
    /// Marks mapped into that texture rather than onto the surface.
    locals_buffer: wgpu::Buffer,
    locals_bind_group: wgpu::BindGroup,
    instances: wgpu::Buffer,
    capacity: usize,
    count: u32,
    chain: crate::BloomChain,
    /// Quarter A (where the vertical blur lands) plus the strength.
    add_bind_group: wgpu::BindGroup,
    strength_buffer: wgpu::Buffer,
    /// The pane's size in device pixels this was built for.
    size: [u32; 2],
    /// Whether `prepare` ran the chain this frame. `paint` draws the halo only
    /// on that, so a frame that skipped the chain — no marks, a strength of 0,
    /// a pane too small to have pixels — cannot lay down the one the frame
    /// before it left in quarter A.
    ready: bool,
    /// The value of [`GlowResources::prepares`] when this copy last prepared,
    /// whether or not it asked for a halo: a strength dialled to 0 and back is
    /// one drag, and the copy is on screen throughout it.
    last_seen: u64,
}

/// Starting size of the instance buffer; it grows by `next_power_of_two` when a
/// frame overflows it. The marks are one per sounding voice.
const INITIAL_DOT_CAPACITY: usize = 64;

impl GlowResources {
    fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let locals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glow_locals_bind_group_layout"),
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
        });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
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
        let filter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glow_filter_bind_group_layout"),
            entries: &[texture_entry(0), sampler_entry(1)],
        });
        // Binding 4 for the strength, matching `AddUniforms` in blit.wgsl —
        // the lattice's own composite holds 3.
        let add_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glow_add_bind_group_layout"),
            entries: &[
                texture_entry(0),
                sampler_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        // The chain overwrites its whole target, so those three take no blend;
        // the one that lands in the egui pass blends the way egui blends.
        let filter =
            |entry| crate::create_post_pipeline(device, entry, target_format, &filter_layout, None);
        GlowResources {
            disc_pipeline: create_disc_pipeline(device, target_format, &locals_layout),
            locals_layout,
            bright_pipeline: filter("fs_bright"),
            downsample_pipeline: filter("fs_blit"),
            blur_h_pipeline: filter("fs_blur_h"),
            blur_v_pipeline: filter("fs_blur_v"),
            add_pipeline: crate::create_post_pipeline(
                device,
                "fs_bloom_add",
                target_format,
                &add_layout,
                Some(EGUI_BLEND),
            ),
            filter_layout,
            add_layout,
            // Linear, and the reason is the halo rather than the marks: the
            // chain reads each target at half the size of the one before it,
            // so every hop is a resample.
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("glow_sampler"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            target_format,
            panes: HashMap::new(),
            prepares: 0,
        }
    }
}

impl GlowPane {
    /// Build the pane's textures for a rect `size` device pixels across. Half
    /// and quarter of THAT, so the halo is a constant share of the pane's own
    /// screen size — the rule the lattice's chain follows, which is what makes
    /// one bloom strength mean the same thing in every picture that grows one.
    fn new(
        device: &wgpu::Device,
        resources: &GlowResources,
        size: [u32; 2],
        prepares: u64,
    ) -> Self {
        let (hw, hh) = (size[0].div_ceil(2).max(1), size[1].div_ceil(2).max(1));
        let discs_view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("glow_discs"),
                size: wgpu::Extent3d { width: hw, height: hh, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: resources.target_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let chain = crate::BloomChain::new(
            device,
            "glow",
            resources.target_format,
            &resources.filter_layout,
            &resources.sampler,
            &discs_view,
            size,
        );
        let locals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glow_locals"),
            size: std::mem::size_of::<GlowUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let strength_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glow_strength"),
            size: std::mem::size_of::<StrengthUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        GlowPane {
            locals_bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("glow_locals_bind_group"),
                layout: &resources.locals_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: locals_buffer.as_entire_binding(),
                }],
            }),
            add_bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("glow_add_bind_group"),
                layout: &resources.add_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&chain.quarter_a_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&resources.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: strength_buffer.as_entire_binding(),
                    },
                ],
            }),
            discs_view,
            locals_buffer,
            instances: create_vertex_buffer::<GlowDot>(device, "glow_dots", INITIAL_DOT_CAPACITY),
            capacity: INITIAL_DOT_CAPACITY,
            count: 0,
            chain,
            strength_buffer,
            size,
            ready: false,
            last_seen: prepares,
        }
    }

    /// Drop every copy that has not prepared for [`PANE_TTL_PREPARES`].
    ///
    /// A copy's id is its surface, and one that is no longer drawn simply stops
    /// calling back — there is no teardown to hang this on, so the copies still
    /// preparing are the only evidence of which ones exist. Run from whichever
    /// copy IS preparing, so a lone survivor still clears the others.
    fn evict_unseen(panes: &mut HashMap<u64, GlowPane>, prepares: u64) {
        panes.retain(|_, pane| prepares.saturating_sub(pane.last_seen) < PANE_TTL_PREPARES);
    }
}

/// The disc pipeline: instanced quads into the offscreen copy, blended the way
/// egui blends its own shapes so marks that overlap accumulate in the copy the
/// way they do in the picture.
fn create_disc_pipeline(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("glow_shader"),
        source: wgpu::ShaderSource::Wgsl(GLOW_SRC.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("glow_pipeline_layout"),
        bind_group_layouts: &[Some(layout)],
        ..Default::default()
    });
    // Same fork egui makes, for the same reason: an sRGB-aware target wants
    // linear values and encodes them itself.
    let shade = if target_format.is_srgb() { "linear" } else { "gamma" };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("glow_disc"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_disc"),
            compilation_options: Default::default(),
            buffers: &[GlowDot::LAYOUT],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(&format!("fs_disc_{shade}")),
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
    })
}

impl CallbackTrait for GlowCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &ScreenDescriptor,
        egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let recreate = callback_resources
            .get::<GlowResources>()
            .is_none_or(|r| r.target_format != self.target_format);
        if recreate {
            callback_resources.insert(GlowResources::new(device, self.target_format));
        }
        let resources: &mut GlowResources =
            callback_resources.get_mut().expect("inserted above when missing");
        resources.prepares = resources.prepares.wrapping_add(1);
        let prepares = resources.prepares;
        GlowPane::evict_unseen(&mut resources.panes, prepares);

        let ppp = screen_descriptor.pixels_per_point.max(f32::EPSILON);
        // The pane's own rect in device pixels, which is what the chain is
        // sized against. Through epaint's own conversion rather than a rounded
        // `width * ppp`, because that conversion is what `paint` stretches the
        // finished halo across: it rounds each EDGE and subtracts, then clamps
        // to the screen, so a rect whose edges round in opposite directions
        // measures a pixel less than its width does, and a rect hanging off the
        // screen measures less again. Sized either way but stretched this way,
        // the halo comes out scaled against the marks it grew from, and slid by
        // whatever the clamp took.
        //
        // A pane thinner than a pixel in either direction has no picture to
        // bloom.
        let viewport = egui::epaint::ViewportInPixels::from_points(
            &self.rect,
            ppp,
            screen_descriptor.size_in_pixels,
        );
        let size = [viewport.width_px.max(0) as u32, viewport.height_px.max(0) as u32];
        let wants = self.strength > 0.0 && !self.dots.is_empty() && size.iter().all(|&d| d > 0);

        // Held whether or not it is wanted this frame, since a strength dialled
        // to 0 and back is one drag: what is skipped is the work, not the
        // textures. They go when the pane's size changes, which is the one
        // thing that invalidates them.
        if resources.panes.get(&self.pane_id).is_some_and(|p| p.size != size) {
            resources.panes.remove(&self.pane_id);
        }
        if !wants {
            if let Some(pane) = resources.panes.get_mut(&self.pane_id) {
                pane.ready = false;
                pane.last_seen = prepares;
            }
            return Vec::new();
        }
        if !resources.panes.contains_key(&self.pane_id) {
            // Built and then stored, rather than assigned in one expression:
            // the constructor reads the layouts and the sampler beside the map
            // it lands in.
            let pane = GlowPane::new(device, resources, size, prepares);
            resources.panes.insert(self.pane_id, pane);
        }

        // Split apart so the pane can be borrowed mutably while the pipelines
        // beside it are still readable.
        let GlowResources {
            disc_pipeline,
            bright_pipeline,
            downsample_pipeline,
            blur_h_pipeline,
            blur_v_pipeline,
            panes,
            ..
        } = resources;
        let pane = panes.get_mut(&self.pane_id).expect("built above when missing");
        pane.last_seen = prepares;

        if self.dots.len() > pane.capacity {
            pane.capacity = self.dots.len().next_power_of_two();
            pane.instances = create_vertex_buffer::<GlowDot>(device, "glow_dots", pane.capacity);
        }
        pane.count = self.dots.len() as u32;
        queue.write_buffer(&pane.instances, 0, bytemuck::cast_slice(&self.dots));
        queue.write_buffer(
            &pane.locals_buffer,
            0,
            bytemuck::bytes_of(&GlowUniforms {
                // The viewport's own edges, back in points: the texture covers
                // exactly the pixels `paint` will lay it over, so the marks in
                // it stand where the marks under it do.
                viewport: [
                    viewport.left_px as f32 / ppp,
                    viewport.top_px as f32 / ppp,
                    size[0] as f32 / ppp,
                    size[1] as f32 / ppp,
                ],
                // Half the pane's size for the copy, so this is what one pixel
                // of THAT target measures in points — twice the display's, and
                // the ramp has to follow it or a small mark comes out at the
                // wrong weight in the halo.
                feather: [2.0 / ppp, 0.0, 0.0, 0.0],
            }),
        );
        queue.write_buffer(
            &pane.strength_buffer,
            0,
            bytemuck::bytes_of(&StrengthUniforms { strength: [self.strength, 0.0, 0.0, 0.0] }),
        );

        // The marks again at half size, and then the lattice's own chain over
        // them, step for step (see [`crate::BloomChain::run`]).
        {
            let mut pass = egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("glow_discs"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &pane.discs_view,
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
            pass.set_pipeline(disc_pipeline);
            pass.set_bind_group(0, &pane.locals_bind_group, &[]);
            pass.set_vertex_buffer(0, pane.instances.slice(..));
            pass.draw(0..4, 0..pane.count);
        }
        pane.chain.run(
            egui_encoder,
            crate::BloomPipelines {
                bright: bright_pipeline,
                downsample: downsample_pipeline,
                blur_h: blur_h_pipeline,
                blur_v: blur_v_pipeline,
            },
            "glow",
        );
        pane.ready = true;

        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<GlowResources>() else {
            return;
        };
        let Some(pane) = resources.panes.get(&self.pane_id).filter(|p| p.ready) else {
            return;
        };
        // Over the pane's rect rather than the surface: quarter A covers
        // exactly that, and stretching it anywhere else would smear the halo.
        // egui-wgpu set this viewport already; it is set again because the
        // callbacks sharing this pass are free to have moved it.
        let vp = info.viewport_in_pixels();
        render_pass.set_viewport(
            vp.left_px as f32,
            vp.top_px as f32,
            vp.width_px as f32,
            vp.height_px as f32,
            0.0,
            1.0,
        );
        render_pass.set_pipeline(&resources.add_pipeline);
        render_pass.set_bind_group(0, &pane.add_bind_group, &[]);
        render_pass.draw(0..4, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_harness::{headless_device, readback, render_to_texture};

    /// A 256x256 test surface at one point per pixel, so a distance in points
    /// is a distance in pixels.
    const SIZE: [u32; 2] = [256, 256];
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

    /// The id a test drawing one halo claims. Which id it is decides nothing —
    /// the map is keyed on it and holds no other — so it stands for "the only
    /// copy on screen" rather than for the Spiral tab's own surface.
    const ONE_PANE: u64 = 0;

    /// A mark in the middle of the surface: bright enough to clear the chain's
    /// threshold, dim enough that brightening it has somewhere to go.
    fn centered_dot() -> GlowDot {
        GlowDot { center: [128.0, 128.0], radius: 12.0, color: [200, 120, 60, 255] }
    }

    /// Run the callback for real — `prepare` then `paint` — over black, and
    /// read the frame back as RGBA8, handing back the resources with it: a
    /// frame drawn at strength 0 is the same bytes however much work went into
    /// it, so whether a chain was built is only visible there.
    fn draw(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        dots: Vec<GlowDot>,
        strength: f32,
    ) -> (Vec<u8>, CallbackResources) {
        // The glow's rect is the whole test surface, so a point is a pixel.
        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32));
        draw_into(device, queue, rect, 1.0, dots, strength)
    }

    /// As [`draw`], over a `rect` in points that need not be the whole surface,
    /// at a display density that need not be one pixel per point.
    ///
    /// Those two are the whole of what the offscreen mapping depends on, and
    /// [`draw`] pins both at their identity — a rect at the origin, one pixel
    /// per point — where an origin never subtracted and a ramp never scaled
    /// both come out right. Production is neither: the editor is Retina and
    /// the Spiral is a docked tab at an offset, and the offline renderer
    /// scales by 1.5 at 1080p and 3.0 at 4K.
    fn draw_into(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rect: egui::Rect,
        ppp: f32,
        dots: Vec<GlowDot>,
        strength: f32,
    ) -> (Vec<u8>, CallbackResources) {
        let cb = GlowCallback { rect, dots, strength, target_format: FORMAT, pane_id: ONE_PANE };
        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: ppp };
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(device, queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));

        let texture = render_to_texture(device, queue, SIZE, FORMAT, wgpu::Color::BLACK, |pass| {
            cb.paint(
                egui::PaintCallbackInfo {
                    viewport: rect,
                    clip_rect: rect,
                    pixels_per_point: ppp,
                    screen_size_px: SIZE,
                },
                pass,
                &resources,
            );
        });
        (readback(device, queue, &texture, SIZE), resources)
    }

    fn pixel(frame: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * SIZE[0] + x) * 4) as usize;
        frame[i..i + 4].try_into().expect("four bytes")
    }

    #[test]
    fn baked_glow_shader_validates() {
        let module = naga::front::wgsl::parse_str(GLOW_SRC)
            .map_err(|e| e.emit_to_string(GLOW_SRC))
            .expect("glow.wgsl must parse");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("glow.wgsl must validate");
        for required in GLOW_ENTRY_POINTS {
            assert!(
                module.entry_points.iter().any(|ep| ep.name == *required),
                "missing entry point `{required}`"
            );
        }
    }

    /// The vertex-layout <-> shader-input contract (attribute locations,
    /// formats, strides), which neither the naga check (shader only) nor the
    /// type system (Rust only) covers — a mismatch otherwise panics at first
    /// paint inside a host.
    #[test]
    fn the_pipelines_build_against_a_headless_device() {
        let Some((device, _queue)) = headless_device() else {
            return;
        };
        let _resources = GlowResources::new(&device, FORMAT);
    }

    /// The halo is LIGHT laid around a mark: it reaches past the mark, and it
    /// takes no alpha away from what is under it.
    ///
    /// The mark itself is the pane's, drawn through egui, so it is not in this
    /// frame at all — which is the point worth holding. What comes back is the
    /// halo alone, over an empty frame, and that is what makes "adds light"
    /// checkable here rather than by subtracting two composites.
    #[test]
    fn the_halo_lands_around_the_mark_and_adds_only_light() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let red = |frame: &[u8], x: u32, y: u32| f32::from(pixel(frame, x, y)[0]);
        let (dark, _) = draw(&device, &queue, vec![centered_dot()], 0.0);
        let (lit, _) = draw(&device, &queue, vec![centered_dot()], 1.5);

        // The mark's edge is at 12 points from the centre; 20 points out is
        // inside the halo's reach and well outside the mark.
        assert_eq!(red(&dark, 148, 128), 0.0, "a strength of 0 lit the frame anyway");
        assert!(red(&lit, 148, 128) > 8.0, "no light beside the mark: {}", red(&lit, 148, 128));
        assert!(
            red(&lit, 128, 128) > 8.0,
            "no light on the mark's own place: {}",
            red(&lit, 128, 128),
        );
        // Far from the mark the halo has run out; the chain blurs at a quarter
        // of a 256-point pane, so 100 points is well past its reach.
        assert_eq!(red(&lit, 250, 10), 0.0, "the halo reached the far corner");
        // Light, not a shape: the halo may never take alpha away from what is
        // under it, and over an opaque frame that means the alpha channel is
        // untouched everywhere.
        for (x, y) in [(128u32, 128u32), (148, 128), (250, 10)] {
            assert_eq!(
                pixel(&lit, x, y)[3],
                255,
                "the halo punched a hole in the alpha at ({x}, {y})",
            );
        }
    }

    /// A strength of 0, and a frame with no marks in it, each skip the whole
    /// thing: no chain is built and no pass runs.
    ///
    /// Asked of the RESOURCES rather than of the frame. Both draw the same
    /// bytes however much work went into them, so a `prepare` that started
    /// running the chain and multiplying by zero would leave the picture
    /// untouched and cost the whole thing — which is what "0 skips it whole" is
    /// a claim about.
    #[test]
    fn nothing_to_light_builds_no_chain() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let built = |dots: Vec<GlowDot>, strength: f32| {
            let (_, resources) = draw(&device, &queue, dots, strength);
            let glow: &GlowResources = resources.get().expect("the callback inserts its resources");
            glow.panes.contains_key(&ONE_PANE)
        };
        assert!(!built(vec![centered_dot()], 0.0), "a strength of 0 built the chain anyway");
        assert!(!built(Vec::new(), 1.5), "an empty frame built the chain anyway");
        assert!(built(vec![centered_dot()], 1.5), "no chain at a strength that asks for one");
    }

    /// A frame that skips the chain paints no halo — including the frame after
    /// one that ran it, whose blurred marks are still sitting in quarter A.
    ///
    /// The textures deliberately outlive the frame that filled them (a strength
    /// dialled to 0 and back is one drag), so "nothing to light" has to be a
    /// decision `paint` makes rather than an empty texture it happens to find.
    #[test]
    fn a_frame_with_nothing_to_light_paints_no_stale_halo() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32));
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        let info = egui::PaintCallbackInfo {
            viewport: rect,
            clip_rect: rect,
            pixels_per_point: 1.0,
            screen_size_px: SIZE,
        };
        let mut resources = CallbackResources::default();
        let prepare = |cb: &GlowCallback, resources: &mut CallbackResources| {
            let mut encoder = device.create_command_encoder(&Default::default());
            let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, resources);
            queue.submit(bufs.into_iter().chain([encoder.finish()]));
        };
        let lit = GlowCallback {
            rect,
            dots: vec![centered_dot()],
            strength: 1.5,
            target_format: FORMAT,
            pane_id: ONE_PANE,
        };
        prepare(&lit, &mut resources);
        // ...and now a frame with the marks gone, over the chain the first one
        // filled.
        let quiet = GlowCallback {
            rect,
            dots: Vec::new(),
            strength: 1.5,
            target_format: FORMAT,
            pane_id: ONE_PANE,
        };
        prepare(&quiet, &mut resources);
        let texture =
            render_to_texture(&device, &queue, SIZE, FORMAT, wgpu::Color::BLACK, |pass| {
                quiet.paint(info, pass, &resources);
            });
        let frame = readback(&device, &queue, &texture, SIZE);
        assert_eq!(
            pixel(&frame, 148, 128),
            [0, 0, 0, 255],
            "the halo of a mark that stopped sounding was painted again",
        );
    }

    /// The halo lands on the mark's own place on a pane that is neither at the
    /// origin nor drawn at one pixel per point — which is every pane the
    /// product actually draws.
    ///
    /// Both terms of the offscreen mapping are exercised only here. The
    /// instances are in SURFACE points and the copy covers the RECT alone, so
    /// `prepare` subtracts the viewport's own top-left before handing the
    /// geometry to the shader; and the copy is half the rect's device size, so
    /// the antialiasing ramp is two display pixels rather than one. Every other
    /// test in this module runs at the identity of both.
    ///
    /// The second assertion is what makes the first mean something. Drop the
    /// origin subtraction and a mark does not vanish — it lands at the fraction
    /// of the rect its SURFACE coordinate happens to name, which for this
    /// fixture is a different place in the same pane, and a test that only
    /// looked for light somewhere would still pass.
    #[test]
    fn the_halo_lands_on_the_marks_own_place_at_an_offset_rect_and_a_retina_scale() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // A 128x128-POINT surface at Retina density, and a pane occupying part
        // of it: points (32,16)..(128,80), which is device pixels
        // (64,32)..(256,160).
        let ppp = 2.0;
        let rect = egui::Rect::from_min_max(egui::pos2(32.0, 16.0), egui::pos2(128.0, 80.0));
        // The middle of that pane, in surface points, as the Spiral hands its
        // dots over.
        let mark = GlowDot { center: [80.0, 48.0], radius: 8.0, color: [200, 120, 60, 255] };
        let (lit, _) = draw_into(&device, &queue, rect, ppp, vec![mark], 1.5);

        // Where the mark stands on screen: its surface point in device pixels.
        let red = |x: u32, y: u32| f32::from(pixel(&lit, x, y)[0]);
        assert!(red(160, 96) > 8.0, "no light on the mark's own place: {}", red(160, 96));

        // ...and where it would land with the origin never subtracted: the
        // mark's surface coordinate read as a fraction of the rect, which is
        // (80/96, 48/64) of a pane starting at (64, 32) device pixels.
        let stray = (224, 128);
        assert_eq!(
            red(stray.0, stray.1),
            0.0,
            "light at {stray:?}, where a mark mapped without the rect's own origin lands",
        );
        // Non-vacuous: the two places are far enough apart that the halo of one
        // cannot reach the other, so the pair of assertions above can disagree.
        assert!(
            (stray.0 as f32 - 160.0).hypot(stray.1 as f32 - 96.0) > 64.0,
            "the fixture put the two readings within one halo of each other",
        );
    }

    /// The chain covers exactly the pixels the halo is stretched across.
    ///
    /// `paint` lays quarter A over `viewport_in_pixels()`, which rounds each
    /// EDGE of the rect and subtracts, then clamps to the screen — not the
    /// width, rounded. On a rect whose two edges round in opposite directions
    /// those differ by a pixel, and a texture sized one way and stretched the
    /// other puts the halo at a slightly different scale from the marks it grew
    /// out of. [`crate::roll`] holds the same property for the same reason.
    #[test]
    fn the_chain_covers_the_pixels_the_halo_is_stretched_across() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let ppp = 2.0;
        // 5.25 -> 10.5 -> 11 (up) and 15.2 -> 30.4 -> 30 (down), so the
        // viewport is 19 px across where the rounded width is 20.
        let rect = egui::Rect::from_min_max(egui::pos2(5.25, 4.0), egui::pos2(15.2, 60.0));
        let mark = GlowDot { center: [10.0, 30.0], radius: 3.0, color: [200, 120, 60, 255] };
        let (_, resources) = draw_into(&device, &queue, rect, ppp, vec![mark], 1.5);

        let vp = egui::epaint::ViewportInPixels::from_points(&rect, ppp, SIZE);
        let glow: &GlowResources = resources.get().expect("prepare inserts its resources");
        let pane = glow.panes.get(&ONE_PANE).expect("a strength of 1.5 asks for a chain");
        assert_eq!(
            pane.size,
            [vp.width_px as u32, vp.height_px as u32],
            "the chain covers {:?} where paint stretches it across {}x{}",
            pane.size,
            vp.width_px,
            vp.height_px,
        );
        // Non-vacuous: the rounded width is not the viewport's width, so a
        // chain sized the tempting way would fail the assertion above.
        assert_ne!(
            vp.width_px as u32,
            (rect.width() * ppp).round() as u32,
            "the fixture's rect no longer rounds its two edges apart",
        );
    }

    /// Two copies in one frame at UNEQUAL rects each keep a chain of their own,
    /// and each paints the halo of its own marks.
    ///
    /// egui-wgpu runs every `prepare` before any `paint`, so one chain between
    /// them is not merely shared: the second prepare rebuilds it at its own
    /// size, and the first then stretches quarter A — laid out in the second
    /// copy's local coordinates — across its own rect. The sizes are the
    /// issue's own measurement (76 then 180 device pixels across one 256-pixel
    /// surface), and unequal is what reaches it: at equal rects a shared chain
    /// is rebuilt at the size it already had and the picture comes out right by
    /// accident.
    #[test]
    fn two_copies_at_unequal_rects_each_keep_their_own_chain() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let tall = SIZE[1] as f32;
        let narrow = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(76.0, tall));
        let wide = egui::Rect::from_min_max(egui::pos2(76.0, 0.0), egui::pos2(256.0, tall));
        // One mark deep in the narrow copy, one high in the wide copy, far
        // enough apart in the surface's own coordinates that neither halo can
        // reach where the other's would land.
        let narrow_mark = GlowDot { center: [20.0, 200.0], ..centered_dot() };
        let wide_mark = GlowDot { center: [106.0, 40.0], ..centered_dot() };
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        let mut resources = CallbackResources::default();
        let callback = |rect, dots, pane_id| GlowCallback {
            rect,
            dots,
            strength: 1.5,
            target_format: FORMAT,
            pane_id,
        };
        let prepare = |cb: &GlowCallback, resources: &mut CallbackResources| {
            let mut encoder = device.create_command_encoder(&Default::default());
            let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, resources);
            queue.submit(bufs.into_iter().chain([encoder.finish()]));
        };

        let first = callback(narrow, vec![narrow_mark], 0);
        let second = callback(wide, vec![wide_mark], 1);
        prepare(&first, &mut resources);
        prepare(&second, &mut resources);

        {
            let glow: &GlowResources = resources.get().expect("prepare inserts its resources");
            let size_of = |id| glow.panes.get(&id).map(|p: &GlowPane| p.size);
            assert_eq!(
                size_of(0),
                Some([76, SIZE[1]]),
                "the narrow copy lost its chain to the wide one",
            );
            assert_eq!(size_of(1), Some([180, SIZE[1]]), "the wide copy has no chain of its own");
        }

        // ...and the narrow copy paints ITS marks. Where the wide copy's mark
        // lands is the whole symptom: through the narrow rect it comes out at
        // that fraction of the narrow copy's own size, which is a reading of
        // 198 with the two sharing a chain and 0 with a chain each.
        let texture =
            render_to_texture(&device, &queue, SIZE, FORMAT, wgpu::Color::BLACK, |pass| {
                first.paint(
                    egui::PaintCallbackInfo {
                        viewport: narrow,
                        clip_rect: narrow,
                        pixels_per_point: 1.0,
                        screen_size_px: SIZE,
                    },
                    pass,
                    &resources,
                );
            });
        let frame = readback(&device, &queue, &texture, SIZE);
        let red = |x: u32, y: u32| f32::from(pixel(&frame, x, y)[0]);
        assert!(red(20, 200) > 8.0, "no light on the narrow copy's own mark: {}", red(20, 200));
        let stray = (13u32, 40u32);
        assert_eq!(
            red(stray.0, stray.1),
            0.0,
            "light at {stray:?}, where the wide copy's mark lands stretched across this rect",
        );
        // Non-vacuous: one halo cannot reach the other's reading.
        assert!(
            (stray.0 as f32 - 20.0).hypot(stray.1 as f32 - 200.0) > 100.0,
            "the fixture put the two readings within one halo of each other",
        );
    }

    /// A copy that stops preparing loses its chain, and the one still preparing
    /// keeps its own.
    ///
    /// The sweep runs from whichever copy IS preparing — nothing here is told
    /// that a pane closed — so the survivor is what makes the eviction happen
    /// at all, and is the reading that would still pass if the sweep took
    /// everything.
    #[test]
    fn a_copy_that_stops_preparing_is_evicted() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(32.0, 32.0));
        let mark = GlowDot { center: [16.0, 16.0], radius: 4.0, color: [200, 120, 60, 255] };
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        let mut resources = CallbackResources::default();
        let prepare = |pane_id, resources: &mut CallbackResources| {
            let cb = GlowCallback {
                rect,
                dots: vec![mark],
                strength: 1.5,
                target_format: FORMAT,
                pane_id,
            };
            let mut encoder = device.create_command_encoder(&Default::default());
            let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, resources);
            queue.submit(bufs.into_iter().chain([encoder.finish()]));
        };
        let live = |resources: &CallbackResources, id| {
            let glow: &GlowResources = resources.get().expect("prepare inserts its resources");
            glow.panes.contains_key(&id)
        };

        prepare(0, &mut resources);
        prepare(1, &mut resources);
        assert!(live(&resources, 0) && live(&resources, 1), "two copies, two chains");

        // Only one of them keeps drawing, past the age the other's chain is
        // held for.
        for _ in 0..PANE_TTL_PREPARES {
            prepare(1, &mut resources);
        }
        assert!(!live(&resources, 0), "the retired copy is still holding a chain");
        assert!(live(&resources, 1), "the sweep took the copy that never stopped drawing");
    }

    /// A pane resized mid-run rebuilds its chain at the new size, and a frame
    /// with more marks than the buffer holds grows it.
    ///
    /// Both are paths a single frame cannot reach — the first needs two
    /// prepares at different rects, the second more marks than
    /// [`INITIAL_DOT_CAPACITY`], which is more voices than the fixture chords
    /// elsewhere in this module carry.
    #[test]
    fn a_resized_pane_rebuilds_and_a_crowded_frame_grows_its_buffer() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        let mut resources = CallbackResources::default();
        let prepare = |dots: Vec<GlowDot>, rect: egui::Rect, resources: &mut CallbackResources| {
            let cb = GlowCallback {
                rect,
                dots,
                strength: 1.5,
                target_format: FORMAT,
                pane_id: ONE_PANE,
            };
            let mut encoder = device.create_command_encoder(&Default::default());
            let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, resources);
            queue.submit(bufs.into_iter().chain([encoder.finish()]));
        };
        let pane_of = |resources: &CallbackResources| {
            let glow: &GlowResources = resources.get().expect("prepare inserts its resources");
            let pane = glow.panes.get(&ONE_PANE).expect("a strength of 1.5 asks for a chain");
            (pane.size, pane.capacity, pane.count)
        };

        let small = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(64.0, 64.0));
        let large = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 120.0));
        prepare(vec![GlowDot { center: [32.0, 32.0], ..centered_dot() }], small, &mut resources);
        assert_eq!(pane_of(&resources), ([64, 64], INITIAL_DOT_CAPACITY, 1));

        // The pane grows — the dock divider dragged, or a render at another
        // aspect — and the chain has to follow, quarter A being what `paint`
        // stretches across the new rect.
        prepare(vec![GlowDot { center: [100.0, 60.0], ..centered_dot() }], large, &mut resources);
        assert_eq!(pane_of(&resources), ([200, 120], INITIAL_DOT_CAPACITY, 1));

        // One more mark than the buffer holds, at the same size, so the regrow
        // is the only thing that moved.
        let crowd: Vec<GlowDot> = (0..INITIAL_DOT_CAPACITY + 1)
            .map(|i| GlowDot { center: [20.0 + i as f32, 60.0], ..centered_dot() })
            .collect();
        let wanted = crowd.len();
        prepare(crowd, large, &mut resources);
        let (size, capacity, count) = pane_of(&resources);
        assert_eq!(size, [200, 120], "a regrow rebuilt the chain");
        assert_eq!(count, wanted as u32, "the frame drew {count} of {wanted} marks");
        assert!(
            capacity >= wanted,
            "the buffer holds {capacity} where the frame handed over {wanted}",
        );
    }
}
