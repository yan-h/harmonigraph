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
//! **One glowing pane per frame, and that is an assumption rather than a
//! guarantee.** The chain and its textures are held once, not per pane id, and
//! egui-wgpu runs every `prepare` before any `paint` — so two of these live in
//! one frame at DIFFERENT sizes tear the chain down and rebuild it twice, and
//! the first pane then stretches the second's halo across its own rect. It is
//! the assumption [`crate::roll`] spends a pane map to avoid, and the roll has
//! to: it has two live copies by construction, the docked pane and the Video
//! tab's preview.
//!
//! What makes it hold HERE is narrower than "one caller", which is not true —
//! the offline renderer draws through `harmonigraph_ui::draw_pane` as well. It
//! is that no arrangement anything but a hand-written `.ron` can express puts
//! two spirals in one frame: the dock holds one Spiral tab, and the Video
//! panel's preview composes a layout that places the lattice and the spectral
//! pane and nothing else. A layout file naming `Spiral` twice is the case that
//! breaks it, and `harmonigraph_ui::draw_pane` already carries the same
//! assumption for the spectrogram's texture slot — fixing either properly means
//! giving that function the placement's index, which is issue #396 and not this
//! module's to decide.
//!
//! **The pane is never evicted, and cannot usefully be.** [`crate::roll`]
//! sweeps its map on a clock of `prepare` calls, which works because a closed
//! roll leaves another one still preparing to sweep with. A hidden Spiral tab
//! stops calling back and nothing else here calls at all, so a TTL would have
//! no clock to age against and would never run. What that costs is one pane's
//! worth of textures — the half-size copy, the thresholded half and the two
//! quarters, about 2.7 MB for a 600x450-point tab on a Retina display — held
//! until the pane is next drawn at a different size. Bounded, and bounded by
//! one rather than by however many tabs were ever opened, which is the
//! difference that makes the roll's sweep worth its map and this one not.

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
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
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
pub fn glow_paint_callback(
    rect: egui::Rect,
    dots: Vec<GlowDot>,
    strength: f32,
    target_format: wgpu::TextureFormat,
) -> egui::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(
        rect,
        GlowCallback { rect, dots, strength, target_format },
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
    /// `None` until a frame asks for a halo, and rebuilt when the pane
    /// resizes — a strength of 0 pays for none of it.
    ///
    /// Held once rather than per pane id, and never swept. Both are the
    /// module head's to justify; the second is why nothing here counts
    /// `prepare` calls the way [`crate::roll`] does.
    pane: Option<GlowPane>,
}

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
        let filter = |entry| {
            crate::create_post_pipeline(device, entry, target_format, &filter_layout, None)
        };
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
            pane: None,
        }
    }
}

impl GlowPane {
    /// Build the pane's textures for a rect `size` device pixels across. Half
    /// and quarter of THAT, so the halo is a constant share of the pane's own
    /// screen size — the rule the lattice's chain follows, which is what makes
    /// one bloom strength mean the same thing in every picture that grows one.
    fn new(device: &wgpu::Device, resources: &GlowResources, size: [u32; 2]) -> Self {
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
        }
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
        if resources.pane.as_ref().is_some_and(|p| p.size != size) {
            resources.pane = None;
        }
        if !wants {
            if let Some(pane) = &mut resources.pane {
                pane.ready = false;
            }
            return Vec::new();
        }
        if resources.pane.is_none() {
            // Built and then stored, rather than assigned in one expression:
            // the constructor reads the layouts and the sampler beside the slot
            // it lands in.
            let pane = GlowPane::new(device, resources, size);
            resources.pane = Some(pane);
        }

        // Split apart so the pane can be borrowed mutably while the pipelines
        // beside it are still readable.
        let GlowResources {
            disc_pipeline,
            bright_pipeline,
            downsample_pipeline,
            blur_h_pipeline,
            blur_v_pipeline,
            pane,
            ..
        } = resources;
        let pane = pane.as_mut().expect("built above when missing");

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
        let Some(pane) = resources.pane.as_ref().filter(|p| p.ready) else {
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
        let cb = GlowCallback { rect, dots, strength, target_format: FORMAT };
        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(device, queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));

        let texture =
            render_to_texture(device, queue, SIZE, FORMAT, wgpu::Color::BLACK, |pass| {
                cb.paint(
                    egui::PaintCallbackInfo {
                        viewport: rect,
                        clip_rect: rect,
                        pixels_per_point: 1.0,
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
            let glow: &GlowResources =
                resources.get().expect("the callback inserts its resources");
            glow.pane.is_some()
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
        };
        prepare(&lit, &mut resources);
        // ...and now a frame with the marks gone, over the chain the first one
        // filled.
        let quiet = GlowCallback { rect, dots: Vec::new(), strength: 1.5, target_format: FORMAT };
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

}
