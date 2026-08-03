//! Haloed label text, drawn as one instanced quad per glyph through a wgpu
//! paint callback.
//!
//! **Why this exists.** A label's rim is the text stamped around two rings,
//! and once the roll and the lattice's fragment shader stopped dominating
//! the frame, those stamps were what was left: turning labels off took the
//! lattice pane's build from 3.9 ms to 0.25 ms, tessellation from 3.5 ms to
//! 0.05 ms, and the frame from 534k vertices to 3.5k. Twenty of every
//! twenty-one stamps were rim. Trimming the ring counts bought a third of
//! that back; this removes the multiplier entirely, so labels can be added
//! wherever they earn their place instead of being rationed.
//!
//! **What it does NOT do.** It does not render text. egui still owns the
//! fonts, the shaping, the layout and the atlas — this takes the glyphs it
//! has already placed ([`GlyphInstance`] carries a glyph's screen rect and
//! its rect in egui's own font atlas) and decides only how they reach the
//! framebuffer. Glyph rasterization is untouched, so a label here is the
//! same pixels as the rest of the UI's text.
//!
//! **Why the rim comes out identical.** Every stamp of a ring is drawn in
//! one color, so their composite is `1 - PRODUCT(1 - alpha * coverage_i)` —
//! a product, which factorizes. That means a fragment can evaluate the whole
//! ring by sampling the glyph's atlas patch at the same offsets, and that
//! per-glyph accumulation composites to exactly what stamping produced, even
//! where neighbouring glyphs' rims overlap. The one thing the arithmetic
//! does NOT excuse is order: stamping laid down every rim before any text,
//! so the rim is drawn as its own pass over all glyphs before the fills.

use std::collections::HashMap;

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};

use crate::{create_vertex_buffer, wgpu};

const TEXT_SRC: &str = include_str!("shaders/text.wgsl");

/// Entry points the text shader must provide.
#[cfg(test)]
pub(crate) const TEXT_ENTRY_POINTS: &[&str] = &["vs_glyph", "fs_rim", "fs_fill"];

/// One glyph: where it goes on screen, where it lives in egui's font atlas,
/// and the two colors it is drawn in.
///
/// Both rects come straight out of the galley egui laid out, so this crate
/// never learns what the text says or which font it is in.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GlyphInstance {
    /// Screen rect of the glyph's ink in points: `[min x, min y, width,
    /// height]`.
    pub rect: [f32; 4],
    /// The same glyph in the atlas, in texels: `[min x, min y, max x, max
    /// y]`. The shader reads nothing outside it — the neighbouring texels
    /// are a different letter.
    pub uv: [f32; 4],
    /// Premultiplied sRGB bytes, straight out of [`egui::Color32`].
    pub fill: [u8; 4],
    /// The rim's color at full strength; the rings decide its opacity. A
    /// fully transparent rim skips the rim pass for this glyph.
    pub rim: [u8; 4],
    /// Where the glyph stands among the nodes of the pane it is drawn over,
    /// as a clip depth: 0 at the near plane, 1 at the far one, which is what
    /// the lattice's depth buffer holds
    /// ([`harmonigraph_scene::Projector::project_with_depth`] is where a
    /// caller gets one). Anything the lattice drew NEARER than this covers
    /// the glyph.
    ///
    /// 0 is a glyph nothing can hide, and is what text with no lattice under
    /// it — the analyzer's names, the learn badge — is drawn at. That is the
    /// meaning of the value rather than a flag: a glyph at the near plane is
    /// in front of every node there is.
    ///
    /// A label belongs slightly in FRONT of the node it names, or it would
    /// be fighting that node's own disc for the pixels it is written on.
    /// Which way to lift it is the caller's business, since only the caller
    /// knows where the camera is; how FAR is bounded below by the difference
    /// between this matrix multiplied on the CPU and on the GPU, and above
    /// by the gap to the next thing that ought to cover the label.
    pub depth: f32,
}

impl GlyphInstance {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<GlyphInstance>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x4, // rect
            1 => Float32x4, // uv
            2 => Unorm8x4,  // fill
            3 => Unorm8x4,  // rim
            4 => Float32,   // depth
        ],
    };
}

/// One ring of the rim: how far out it sits (points), how opaque each stamp
/// is, and how many stamps go round it.
///
/// The look lives in the UI layer, which owns what a label should look like;
/// this crate is handed the numbers. Samples of 0 is a ring that isn't
/// there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextRing {
    pub radius: f32,
    pub alpha: f32,
    pub samples: u32,
}

/// egui's font atlas, as this crate needs it: the pixels, and a key that
/// changes whenever they do.
///
/// A callback cannot reach the texture egui uploaded — `CallbackResources`
/// holds what WE put there — so the atlas is mirrored. The key is what makes
/// that affordable: the mirror is re-uploaded only when it moves, which is
/// when a glyph nobody has drawn before is rasterized.
pub struct FontAtlas {
    pub image: std::sync::Arc<egui::ColorImage>,
    pub key: u64,
}

/// Draw `glyphs` into `rect`. `pane_id` must be unique per pane drawing text
/// in the same frame (each keeps its own instance buffer; the pipeline and
/// the atlas are shared).
///
/// `atlas` is `None` on the frames where egui's atlas has not changed, which
/// is nearly all of them.
///
/// `occluder` is the id of the LATTICE pane (the one
/// [`crate::lattice_paint_callback`] was given) whose nodes may cover this
/// text, and `rect` is then that pane's own rect — the two describe one
/// picture. `None` for text drawn over anything else, which is then drawn
/// over everything, as all of it was before.
pub fn text_paint_callback(
    rect: egui::Rect,
    glyphs: Vec<GlyphInstance>,
    rings: [TextRing; 2],
    atlas: Option<FontAtlas>,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
    occluder: Option<u64>,
) -> egui::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(
        rect,
        TextCallback { rect, glyphs, rings, atlas, target_format, pane_id, occluder },
    )
}

struct TextCallback {
    /// Where the text is drawn, which is also the rect the `occluder` depth
    /// buffer covers. Kept rather than read back off `PaintCallbackInfo`,
    /// since the uniforms carrying it are written in `prepare`.
    rect: egui::Rect,
    glyphs: Vec<GlyphInstance>,
    rings: [TextRing; 2],
    atlas: Option<FontAtlas>,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
    occluder: Option<u64>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TextUniforms {
    screen_points: [f32; 2],
    atlas_size: [f32; 2],
    pixels_per_point: f32,
    /// WGSL aligns a `vec4<f32>` to 16 bytes, so the rings start at 32 and
    /// this is the gap in front of them. Named rather than derived because
    /// the mismatch is a validation error at first paint, not a compile one.
    _pad: [f32; 3],
    ring0: [f32; 4],
    ring1: [f32; 4],
    /// The rect the occluding depth buffer covers, in points: min, then
    /// size. A zero size means there is no occluder, and is what the shader
    /// reads before it touches the texture.
    pane: [f32; 4],
}

struct TextResources {
    /// The rim pass and the fill pass: one shader, one vertex layout, two
    /// fragment entry points. Two pipelines rather than one with a flag,
    /// because the pass is a property of the draw and not of any glyph.
    rim_pipeline: wgpu::RenderPipeline,
    fill_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    target_format: wgpu::TextureFormat,
    /// The mirrored font atlas, and the key of what is in it.
    atlas: Option<wgpu::Texture>,
    atlas_size: [u32; 2],
    atlas_key: u64,
    /// Stands in for a lattice depth buffer where there is none: one texel
    /// at the far plane, so every glyph reads as unoccluded. The binding is
    /// unconditional — a bind group layout cannot have a hole in it — and
    /// this is what fills it.
    fallback_depth: wgpu::TextureView,
    panes: HashMap<u64, TextPane>,
}

struct TextPane {
    uniform_buffer: wgpu::Buffer,
    bind_group: Option<wgpu::BindGroup>,
    instance_buffer: wgpu::Buffer,
    capacity: usize,
    count: u32,
    /// The depth buffer this pane's bind group names, and the epoch of the
    /// texture behind it — [`NO_OCCLUDER`] for the fallback. A pane resize
    /// builds a fresh offscreen target, and a bind group still naming the
    /// old one is a destroyed texture.
    depth: wgpu::TextureView,
    depth_epoch: u64,
}

/// [`TextPane::depth_epoch`] for a pane bound to the fallback. Not an epoch
/// any [`crate::Offscreen`] can be handed: they count up from zero.
const NO_OCCLUDER: u64 = u64::MAX;

/// Starting size of a pane's glyph buffer. A lattice full of labels is a few
/// thousand glyphs; it grows by `next_power_of_two` when a frame overflows.
const INITIAL_GLYPH_CAPACITY: usize = 2048;

impl TextResources {
    /// `encoder` is used once, to clear the fallback depth texel to the far
    /// plane. A depth texture cannot be written from the CPU, and a texel
    /// left at wgpu's zero fill would read as the NEAR plane — every label
    /// on every pane with no lattice under it, gone.
    fn new(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("text_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // Read with `textureLoad` at a whole texel, so it needs no
                // sampler of its own — which is just as well, since a depth
                // format is not filterable and the one above is.
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let rim_pipeline = create_text_pipeline(device, target_format, &layout, "fs_rim");
        let fill_pipeline = create_text_pipeline(device, target_format, &layout, "fs_fill");
        // Linear, to match how egui samples the same atlas. At the sizes
        // labels are drawn the glyph lands texel for texel on the
        // framebuffer, so this is an identity for the fill and only does
        // real work for the rim's off-grid taps.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("text_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        TextResources {
            rim_pipeline,
            fill_pipeline,
            layout,
            sampler,
            target_format,
            atlas: None,
            atlas_size: [0, 0],
            atlas_key: u64::MAX,
            fallback_depth: far_texel(device, encoder),
            panes: HashMap::new(),
        }
    }

    /// Upload egui's atlas into our own texture, recreating it when it has
    /// grown — and carrying every pane already prepared this frame over onto
    /// the new texture, since none of them will prepare again before they are
    /// painted.
    ///
    /// That last part is the whole of why this is not four lines. egui-wgpu
    /// runs EVERY callback's `prepare` and only then every `paint`, and which
    /// pane brings a changed atlas is decided by which pane happened to lay out
    /// a glyph nobody had drawn before — the roll scrolling a new name in, a
    /// lattice node crossing onto a new rung of the size ladder. Every pane
    /// that flushed BEFORE that one has already had its turn:
    ///
    ///   - its bind group names the texture being replaced here, and a pane
    ///     whose bind group is dropped paints nothing at all — a whole pane's
    ///     text gone for one frame, which on a pane that is scrolling reads as
    ///     the text flickering;
    ///   - its uniforms carry the atlas size it prepared against, and the
    ///     shader normalizes texels by that, so a pane left holding the old one
    ///     would sample a fraction of the way into the wrong row.
    ///
    /// Both are put right here rather than deferred to a next frame that, for
    /// those panes, comes after they have been drawn. Their instance data needs
    /// nothing, and that rests on two things outside this function. The mirror
    /// above it (`harmonigraph_ui::text::atlas_if_changed`) records a pane's
    /// glyphs as seen BEFORE it decides whether to hand out an atlas at all, so
    /// a pane told `None` is one whose every texel already sits in a snapshot
    /// some earlier flush uploaded. And an atlas that GREW keeps those texels
    /// where they were, epaint only ever doubling the height and appending
    /// transparent rows — which is also what makes the atlas monotone across a
    /// frame, so no later pane can bring a SMALLER one and strand an earlier
    /// pane's uvs off the end of it.
    ///
    /// The one arrangement neither covers is epaint recycling texels in place,
    /// which it does once a glyph is too big for the atlas to grow for: live
    /// glyphs move at a CONSTANT size, so nothing is recreated and nothing here
    /// can notice. That is held off upstream by `MAX_GLYPH_PX`, and is the
    /// reason that ceiling is a ceiling rather than a preference.
    fn mirror_atlas(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, atlas: &FontAtlas) {
        let size = [atlas.image.width() as u32, atlas.image.height() as u32];
        let recreated = self.atlas.is_none() || self.atlas_size != size;
        if recreated {
            self.atlas = Some(device.create_texture(&wgpu::TextureDescriptor {
                label: Some("text_font_atlas"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }));
            self.atlas_size = size;
        }
        let texture = self.atlas.as_ref().expect("created above");
        queue.write_texture(
            texture.as_image_copy(),
            bytemuck::cast_slice(atlas.image.as_raw()),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] * 4),
                rows_per_image: Some(size[1]),
            },
            wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        );
        self.atlas_key = atlas.key;
        if !recreated {
            return;
        }
        let view = texture.create_view(&Default::default());
        let (layout, sampler) = (&self.layout, &self.sampler);
        for pane in self.panes.values_mut() {
            // The size alone, not the whole struct: a pane that has already
            // prepared wrote the rest of its uniforms this frame, and one that
            // has not is about to write all of them including this.
            queue.write_buffer(
                &pane.uniform_buffer,
                ATLAS_SIZE_OFFSET,
                bytemuck::cast_slice(&[size[0] as f32, size[1] as f32]),
            );
            pane.bind_group = Some(bind_group(device, layout, sampler, &view, pane));
        }
    }
}

/// A 1x1 depth texture cleared to the far plane — [`TextResources`]'s stand-in
/// for a lattice that is not there.
///
/// Cleared by a render pass because that is the only way to put a value in a
/// depth texture: `Depth32Float` takes no buffer copy, so there is no
/// `write_texture` route to it.
fn far_texel(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) -> wgpu::TextureView {
    let view = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("text_no_occluder"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default());
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("text_no_occluder_clear"),
        color_attachments: &[],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    view
}

/// Where [`TextUniforms::atlas_size`] sits, for the partial write above.
/// Taken from the type rather than counted, so reordering the struct cannot
/// leave this pointing at `screen_points`.
const ATLAS_SIZE_OFFSET: wgpu::BufferAddress =
    std::mem::offset_of!(TextUniforms, atlas_size) as wgpu::BufferAddress;

/// One pane's bind group: its own uniforms and its own occluder, plus the
/// shared atlas and sampler.
fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    view: &wgpu::TextureView,
    pane: &TextPane,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("text_bind_group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: pane.uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&pane.depth),
            },
        ],
    })
}

/// One pass's pipeline: instanced quads blended exactly the way egui blends
/// its own text, so a label composites over the picture identically to the
/// stamped version it replaces.
fn create_text_pipeline(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    layout: &wgpu::BindGroupLayout,
    fragment: &str,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("text_shader"),
        source: wgpu::ShaderSource::Wgsl(TEXT_SRC.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("text_pipeline_layout"),
        bind_group_layouts: &[Some(layout)],
        ..Default::default()
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(fragment),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_glyph"),
            compilation_options: Default::default(),
            buffers: &[GlyphInstance::LAYOUT],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(fragment),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                }),
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

impl CallbackTrait for TextCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &ScreenDescriptor,
        egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let recreate = callback_resources
            .get::<TextResources>()
            .is_none_or(|r| r.target_format != self.target_format);
        if recreate {
            callback_resources.insert(TextResources::new(device, egui_encoder, self.target_format));
        }
        // Taken while the lattice's resources are borrowed, and finished with
        // before this batch's own are: two entries of one map cannot be held
        // at once. Cloning a view is cloning a handle.
        let occluder = self
            .occluder
            .and_then(|id| crate::lattice_occluder(callback_resources, id));
        let resources: &mut TextResources =
            callback_resources.get_mut().expect("inserted above when missing");

        if let Some(atlas) = self.atlas.as_ref().filter(|a| a.key != resources.atlas_key) {
            resources.mirror_atlas(device, queue, atlas);
        }
        // No atlas yet means the first frame arrived without one: nothing can
        // be drawn, and the next frame that sees a change will bring it.
        if resources.atlas.is_none() {
            return Vec::new();
        }

        let ppp = screen_descriptor.pixels_per_point.max(f32::EPSILON);
        let ring = |r: TextRing| [r.radius, r.alpha, r.samples as f32, 0.0];
        // A zero size where there is no lattice to be hidden behind — the
        // shader reads it before it reads the texture.
        let pane_rect = match &occluder {
            Some(_) => [
                self.rect.min.x,
                self.rect.min.y,
                self.rect.width(),
                self.rect.height(),
            ],
            None => [0.0; 4],
        };
        let uniforms = TextUniforms {
            screen_points: [
                screen_descriptor.size_in_pixels[0] as f32 / ppp,
                screen_descriptor.size_in_pixels[1] as f32 / ppp,
            ],
            atlas_size: [resources.atlas_size[0] as f32, resources.atlas_size[1] as f32],
            pixels_per_point: ppp,
            _pad: [0.0; 3],
            ring0: ring(self.rings[0]),
            ring1: ring(self.rings[1]),
            pane: pane_rect,
        };

        let view = resources
            .atlas
            .as_ref()
            .expect("checked above")
            .create_view(&Default::default());
        let (depth, epoch) = match occluder {
            Some((view, epoch)) => (view, epoch),
            None => (resources.fallback_depth.clone(), NO_OCCLUDER),
        };
        let (layout, sampler) = (&resources.layout, &resources.sampler);
        let pane = resources.panes.entry(self.pane_id).or_insert_with(|| TextPane {
            depth: depth.clone(),
            depth_epoch: epoch,
            uniform_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("text_uniforms"),
                size: std::mem::size_of::<TextUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            bind_group: None,
            instance_buffer: create_vertex_buffer::<GlyphInstance>(
                device,
                "text_glyphs",
                INITIAL_GLYPH_CAPACITY,
            ),
            capacity: INITIAL_GLYPH_CAPACITY,
            count: 0,
        });
        // A pane whose occluder has changed hands — a lattice that started or
        // stopped drawing, or a resize that built a new offscreen target —
        // needs a bind group naming the new one. Compared by epoch rather
        // than by the view, which carries no identity to compare.
        if pane.depth_epoch != epoch {
            pane.depth = depth;
            pane.depth_epoch = epoch;
            pane.bind_group = None;
        }
        if pane.bind_group.is_none() {
            pane.bind_group = Some(bind_group(device, layout, sampler, &view, pane));
        }

        if self.glyphs.len() > pane.capacity {
            pane.capacity = self.glyphs.len().next_power_of_two();
            pane.instance_buffer =
                create_vertex_buffer::<GlyphInstance>(device, "text_glyphs", pane.capacity);
        }
        pane.count = self.glyphs.len() as u32;
        if !self.glyphs.is_empty() {
            queue.write_buffer(&pane.instance_buffer, 0, bytemuck::cast_slice(&self.glyphs));
        }
        queue.write_buffer(&pane.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<TextResources>() else {
            return;
        };
        let Some(pane) = resources.panes.get(&self.pane_id) else {
            return;
        };
        let Some(bind_group) = pane.bind_group.as_ref() else {
            return;
        };
        if pane.count == 0 {
            return;
        }
        // Against the whole surface, as the roll does: the geometry is in
        // screen points, so the clip mapping is egui's own. The scissor
        // egui-wgpu set from the clip rect is left alone, and is what keeps
        // a label inside its pane.
        render_pass.set_viewport(
            0.0,
            0.0,
            info.screen_size_px[0] as f32,
            info.screen_size_px[1] as f32,
            0.0,
            1.0,
        );
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
        // Every rim, then every fill. Stamping had that order for free (it
        // drew the rings before the text); here it is two draws, and without
        // it two neighbouring letters darken each other's ink.
        render_pass.set_pipeline(&resources.rim_pipeline);
        render_pass.draw(0..4, 0..pane.count);
        render_pass.set_pipeline(&resources.fill_pipeline);
        render_pass.draw(0..4, 0..pane.count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{headless_device, readback, render_to_texture};

    const SIZE: [u32; 2] = [64, 64];
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

    #[test]
    fn baked_text_shader_validates() {
        let module = naga::front::wgsl::parse_str(TEXT_SRC)
            .map_err(|e| e.emit_to_string(TEXT_SRC))
            .expect("text.wgsl must parse");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("text.wgsl must validate");
        for required in TEXT_ENTRY_POINTS {
            assert!(
                module.entry_points.iter().any(|ep| ep.name == *required),
                "missing entry point `{required}`"
            );
        }
    }

    #[test]
    fn the_pipelines_build_against_a_headless_device() {
        let Some((device, _queue)) = headless_device() else {
            return;
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        let _resources = TextResources::new(&device, &mut encoder, FORMAT);
    }

    /// A stand-in atlas: one opaque 8x8 "glyph" at (8, 8), with nothing
    /// around it. Coverage is the alpha channel, as egui's atlas stores it.
    fn atlas() -> FontAtlas {
        let mut image = egui::ColorImage::filled([32, 32], egui::Color32::TRANSPARENT);
        for y in 8..16 {
            for x in 8..16 {
                image[(x, y)] = egui::Color32::WHITE;
            }
        }
        FontAtlas { image: std::sync::Arc::new(image), key: 1 }
    }

    /// The whole target, which is the rect every fixture here draws into.
    fn whole() -> egui::Rect {
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32))
    }

    /// Draw one glyph through both passes and read the frame back.
    fn draw(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        glyph: GlyphInstance,
        rings: [TextRing; 2],
    ) -> Vec<u8> {
        let cb = TextCallback {
            rect: whole(),
            glyphs: vec![glyph],
            rings,
            atlas: Some(atlas()),
            target_format: FORMAT,
            pane_id: 0,
            occluder: None,
        };
        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(device, queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));

        let rect = whole();
        let texture = render_to_texture(
            device,
            queue,
            SIZE,
            FORMAT,
            wgpu::Color::TRANSPARENT,
            |pass| {
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
            },
        );
        readback(device, queue, &texture, SIZE)
    }

    fn pixel(frame: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * SIZE[0] + x) * 4) as usize;
        [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
    }

    /// A glyph 8 points wide at (24, 24), reading the 8x8 patch of [`atlas`].
    fn glyph() -> GlyphInstance {
        GlyphInstance {
            rect: [24.0, 24.0, 8.0, 8.0],
            uv: [8.0, 8.0, 16.0, 16.0],
            fill: [255, 255, 255, 255],
            rim: [255, 0, 0, 255],
            depth: 0.0,
        }
    }

    /// The glyph lands where it was told to, in its own color, and the rim
    /// stands outside it in the rim's color — the whole contract in one
    /// picture.
    #[test]
    fn a_glyph_paints_its_ink_and_the_rim_stands_outside_it() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let rings = [
            TextRing { radius: 0.0, alpha: 0.0, samples: 0 },
            TextRing { radius: 2.0, alpha: 1.0, samples: 8 },
        ];
        let frame = draw(&device, &queue, glyph(), rings);
        assert_eq!(pixel(&frame, 28, 28), [255, 255, 255, 255], "the glyph itself");
        assert_eq!(pixel(&frame, 23, 28), [255, 0, 0, 255], "the rim, one point out");
        assert_eq!(pixel(&frame, 21, 28), [0, 0, 0, 0], "nothing past the rim's radius");
        assert_eq!(pixel(&frame, 4, 4), [0, 0, 0, 0], "nothing anywhere else");
    }

    /// A pane whose text is already prepared keeps it when a LATER pane in the
    /// same frame brings a grown atlas.
    ///
    /// Which pane brings one is not a property of the pane: it is whichever
    /// happened to lay out a glyph nobody had drawn before, so on any frame the
    /// panes ahead of it in paint order have already had their `prepare` and
    /// will not get another before they are painted. Both halves of what
    /// [`TextResources::mirror_atlas`] hands them are checked here, and each
    /// fails on its own — dropping the bind group paints nothing at all, and
    /// leaving the old `atlas_size` in the uniforms normalizes the glyph's
    /// texels by the wrong height, which lands the sample below the patch,
    /// where the atlas is empty. Both are a pane's whole text gone for a frame,
    /// and on a pane that is scrolling that is text flickering.
    ///
    /// The last frame is what makes the second one mean anything. A pane left
    /// holding BOTH its old bind group and its old uniforms is stale but
    /// self-consistent: it samples the retired texture by the size that texture
    /// really is, so it draws the right pixels and passes any assertion about
    /// the frame it was stranded on. What it can never do is read a glyph
    /// rasterized into the region the atlas grew INTO, since its texture stops
    /// short of it — and it never recovers on its own, because `prepare`
    /// rebuilds a bind group only when there is none. So the third frame draws
    /// off a patch that exists solely in the grown half, which no pane still
    /// bound to the old texture can reach.
    #[test]
    fn a_prepared_pane_survives_a_later_pane_growing_the_atlas() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // An opaque 8x8 patch at (8, 8), and — once the atlas is tall enough to
        // hold one — a second at (8, 40), which is reachable only through a
        // texture of the grown size. Everything else stays transparent, so a
        // glyph normalized by the wrong height samples nothing rather than
        // something plausible: the stale size puts this fixture's probe at texel
        // 25, which is inside the ORIGINAL rows and below the first patch.
        let atlas_of = |height: usize, key: u64| {
            let mut image = egui::ColorImage::filled([32, height], egui::Color32::TRANSPARENT);
            for top in [8, GROWN_PATCH_TOP] {
                if top + 8 > height {
                    continue;
                }
                for y in top..top + 8 {
                    for x in 8..16 {
                        image[(x, y)] = egui::Color32::WHITE;
                    }
                }
            }
            FontAtlas { image: std::sync::Arc::new(image), key }
        };
        let bare = [
            TextRing { radius: 0.0, alpha: 0.0, samples: 0 },
            TextRing { radius: 0.0, alpha: 0.0, samples: 0 },
        ];
        let at = |x: f32, pane_id: u64, atlas: Option<FontAtlas>| TextCallback {
            rect: whole(),
            glyphs: vec![GlyphInstance { rect: [x, 24.0, 8.0, 8.0], ..glyph() }],
            rings: bare,
            atlas,
            target_format: FORMAT,
            pane_id,
            occluder: None,
        };

        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        let rect = whole();
        // One frame, in egui-wgpu's own order: every `prepare`, then every
        // `paint`. That order is the whole mechanism — a pane cannot repair
        // itself between the two.
        let frame = |resources: &mut CallbackResources, callbacks: [TextCallback; 2]| -> Vec<u8> {
            let mut encoder = device.create_command_encoder(&Default::default());
            let mut buffers = Vec::new();
            for callback in &callbacks {
                buffers.extend(callback.prepare(&device, &queue, &screen, &mut encoder, resources));
            }
            queue.submit(buffers.into_iter().chain([encoder.finish()]));
            let texture =
                render_to_texture(&device, &queue, SIZE, FORMAT, wgpu::Color::TRANSPARENT, |pass| {
                    for callback in &callbacks {
                        callback.paint(
                            egui::PaintCallbackInfo {
                                viewport: rect,
                                clip_rect: rect,
                                pixels_per_point: 1.0,
                                screen_size_px: SIZE,
                            },
                            pass,
                            resources,
                        );
                    }
                });
            readback(&device, &queue, &texture, SIZE)
        };

        // The first pane brings the atlas and the second rides on it, which is
        // the ordinary frame and the baseline the second one is read against.
        let first =
            frame(&mut resources, [at(8.0, 0, Some(atlas_of(32, 1))), at(40.0, 1, None)]);
        assert_eq!(pixel(&first, 12, 28), [255, 255, 255, 255], "the leading pane's glyph");
        assert_eq!(pixel(&first, 44, 28), [255, 255, 255, 255], "the trailing pane's glyph");

        // Now the other way round: the leading pane has nothing new to say and
        // the trailing one grows the atlas out from under it.
        let grown =
            frame(&mut resources, [at(8.0, 0, None), at(40.0, 1, Some(atlas_of(64, 2)))]);
        assert_eq!(
            pixel(&grown, 12, 28),
            [255, 255, 255, 255],
            "the leading pane's text must survive the atlas growing after it prepared",
        );
        assert_eq!(pixel(&grown, 44, 28), [255, 255, 255, 255], "the trailing pane's glyph");

        // And the leading pane is on the NEW texture, not merely coherent with
        // the old one: it draws off the patch that only the grown atlas holds.
        // The trailing pane re-uploads at the SAME size here, which is the
        // ordinary case — a glyph packed into space the atlas already had — and
        // takes `mirror_atlas`'s early return, so nothing is handed to the
        // leading pane on this frame either.
        let reaching = GlyphInstance {
            rect: [8.0, 24.0, 8.0, 8.0],
            uv: [8.0, GROWN_PATCH_TOP as f32, 16.0, GROWN_PATCH_TOP as f32 + 8.0],
            ..glyph()
        };
        let deeper = frame(&mut resources, [
            TextCallback {
                rect: whole(),
                glyphs: vec![reaching],
                rings: bare,
                atlas: None,
                target_format: FORMAT,
                pane_id: 0,
                occluder: None,
            },
            at(40.0, 1, Some(atlas_of(64, 3))),
        ]);
        assert_eq!(
            pixel(&deeper, 12, 28),
            [255, 255, 255, 255],
            "the leading pane must be reading the grown texture, not the retired one",
        );
        assert_eq!(pixel(&deeper, 44, 28), [255, 255, 255, 255], "the trailing pane's glyph");
    }

    /// Where the second patch of the fixture above sits: past the 32 rows the
    /// atlas starts with, so only a texture of the grown size holds it.
    const GROWN_PATCH_TOP: usize = 40;

    /// Where the pair of nodes below stands, in world units. Off-center in
    /// both axes on purpose: a label asks the depth buffer about the pixel it
    /// is being drawn on, and a lookup that flipped or transposed that pixel
    /// would still be right in the middle of the picture.
    const STACK_AT: glam::Vec2 = glam::Vec2::new(0.7, -0.5);

    /// The pane this scene is drawn into. Bigger than the fixtures above,
    /// because this one is about a node's DISC rather than about a glyph: at
    /// the real ratio of node radius to lattice spacing, 64 points across
    /// puts the whole disc inside a couple of pixels.
    const SCENE_SIZE: [u32; 2] = [256, 256];

    /// Two nodes on the same pixels, one sevens step apart, and nothing else.
    /// Face-on and orthographic, which is the arrangement that puts one node
    /// squarely behind another; a sheared or orbited view only spreads the
    /// same overlap out.
    ///
    /// The radius against the step is the lattice's own
    /// (`NODE_RADIUS_FACTOR`), so the lift a label is given here is the share
    /// of the gap it is given in the picture.
    fn one_node_behind_another() -> harmonigraph_scene::Scene {
        let mut scene = crate::tests::parity_scene();
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.node_radius = 0.25;
        let mut near = scene.nodes[0];
        near.world_pos = STACK_AT.extend(0.0);
        near.activation = 1.0;
        near.scale = 1.0;
        near.gutter = 0.0;
        near.hovered = false;
        near.on_home = true;
        let mut far = near;
        far.world_pos = STACK_AT.extend(-1.0);
        far.on_home = false;
        scene.nodes = vec![near, far];
        // The grid would draw across the same pixels, and whether IT covers a
        // label is a separate question with its own answer (`create_pipelines`).
        scene.grid.clear();
        scene
    }

    /// A node in front covers the label of the node behind it, the way it
    /// covers that node itself.
    ///
    /// This is the whole feature in one picture, and it is an end-to-end
    /// test on purpose: the lattice's own pass writes the depth, the label
    /// pass reads it, and the two agree only if the depth a label is handed
    /// on the CPU means the same thing as the depth a node's vertices arrive
    /// at on the GPU. Nothing short of running both passes checks that.
    ///
    /// The near label is the half that would fail quietly. A lift too small
    /// to clear the difference between those two multiplications leaves a
    /// name fighting the disc it is written on, which is a label flickering
    /// against itself rather than a label in the wrong place.
    #[test]
    fn a_nearer_node_covers_the_label_of_the_node_behind() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let scene = one_node_behind_another();
        let points = egui::vec2(SCENE_SIZE[0] as f32, SCENE_SIZE[1] as f32);
        let projector = scene.projector(glam::Vec2::new(points.x, points.y));
        let depth_of = |node: &harmonigraph_scene::NodeInstance| {
            scene
                .label_depth(&projector, node.world_pos)
                .expect("both nodes are in front of the camera")
        };
        let (near, far) = (depth_of(&scene.nodes[0]), depth_of(&scene.nodes[1]));
        assert!(near < far, "the fixture's near node must be nearer: {near} against {far}");

        // The pixel both discs are painted on, which is where the label goes
        // and where it is read back. Off-center, so a depth lookup that
        // flipped or transposed the pane would sample bare background and
        // find nothing to be hidden behind.
        let on = projector
            .project(scene.nodes[0].world_pos)
            .expect("the stack is in front of the camera");
        let (x, y) = (on.x.round(), on.y.round());
        assert!(
            (x - points.x / 2.0).abs() > 8.0 && (y - points.y / 2.0).abs() > 8.0,
            "the fixture's nodes must sit off-center, at ({x}, {y}) of {points:?}",
        );

        // One glyph on that pixel. No rim: the fill alone answers the
        // question, and a rim would spread the reading over pixels the
        // discard is not being asked about.
        let bare = [
            TextRing { radius: 0.0, alpha: 0.0, samples: 0 },
            TextRing { radius: 0.0, alpha: 0.0, samples: 0 },
        ];
        let at = [x - 4.0, y - 4.0, 8.0, 8.0];
        const LATTICE: u64 = 3;
        let label = |depth: f32, occluder: Option<u64>| -> [u8; 4] {
            let lattice = crate::LatticeCallback::from_scene(&scene, points, FORMAT, LATTICE, None);
            let text = TextCallback {
                rect: egui::Rect::from_min_size(egui::Pos2::ZERO, points),
                glyphs: vec![GlyphInstance { rect: at, depth, ..glyph() }],
                rings: bare,
                atlas: Some(atlas()),
                target_format: FORMAT,
                pane_id: 0,
                occluder,
            };
            let mut resources = CallbackResources::default();
            let screen =
                ScreenDescriptor { size_in_pixels: SCENE_SIZE, pixels_per_point: 1.0 };
            let mut encoder = device.create_command_encoder(&Default::default());
            // In egui-wgpu's order: the lattice fills the depth buffer the
            // labels are about to read, on the same encoder.
            let mut buffers =
                lattice.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
            buffers.extend(text.prepare(&device, &queue, &screen, &mut encoder, &mut resources));
            queue.submit(buffers.into_iter().chain([encoder.finish()]));

            // The lattice is NOT composited here: what the target holds is
            // the label alone, so a pixel says drawn or not drawn without
            // anything to read it against.
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, points);
            let texture = render_to_texture(
                &device,
                &queue,
                SCENE_SIZE,
                FORMAT,
                wgpu::Color::TRANSPARENT,
                |pass| {
                    text.paint(
                        egui::PaintCallbackInfo {
                            viewport: rect,
                            clip_rect: rect,
                            pixels_per_point: 1.0,
                            screen_size_px: SCENE_SIZE,
                        },
                        pass,
                        &resources,
                    );
                },
            );
            let frame = readback(&device, &queue, &texture, SCENE_SIZE);
            let i = ((y as u32 * SCENE_SIZE[0] + x as u32) * 4) as usize;
            [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
        };

        assert_eq!(
            label(far, Some(LATTICE)),
            [0, 0, 0, 0],
            "the far node's label must be cut by the node drawn in front of it",
        );
        assert_eq!(
            label(near, Some(LATTICE)),
            [255, 255, 255, 255],
            "the near node's label must survive its OWN disc",
        );
        // The same glyph with nothing to be hidden behind, which is what
        // makes the first assertion a reading of the occluder rather than of
        // some other reason a glyph might not have drawn.
        assert_eq!(
            label(far, None),
            [255, 255, 255, 255],
            "a batch with no occluder must draw over everything, as it always has",
        );
    }

    /// The rim's opacity is `1 - PRODUCT(1 - alpha)` over the samples that
    /// cover a pixel, which is what stamping the text around that ring
    /// composites to. Checked where exactly one sample can reach: two
    /// stamps at half alpha must read 75% opaque, not 50% and not 100%.
    ///
    /// This is the claim the whole approach rests on — that the rim was
    /// re-derived rather than re-invented — so it is measured against the
    /// arithmetic rather than eyeballed.
    #[test]
    fn the_rim_accumulates_the_way_stamping_composited() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // Two samples at half alpha, two points either side. The glyph is 8
        // points wide, so a pixel in the middle of it is covered by both and
        // a pixel near its left edge only by the one reaching in from the
        // right — the two cases the arithmetic has to tell apart.
        let rings = [
            TextRing { radius: 0.0, alpha: 0.0, samples: 0 },
            TextRing { radius: 2.0, alpha: 0.5, samples: 2 },
        ];
        let frame = draw(&device, &queue, GlyphInstance { fill: [0, 0, 0, 0], ..glyph() }, rings);
        let both = pixel(&frame, 28, 28);
        assert!(
            both[3].abs_diff(191) <= 2,
            "two half-alpha samples should compose to 75%, got {both:?}",
        );
        let one = pixel(&frame, 25, 28);
        assert!(
            one[3].abs_diff(128) <= 2,
            "one half-alpha sample should read 50%, got {one:?}",
        );
    }
}
