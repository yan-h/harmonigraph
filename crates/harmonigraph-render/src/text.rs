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
//! **Who else draws through it.** The pipelines, the atlas mirror and the
//! shader are shared with the lattice, which draws its node names inside its
//! own scene pass rather than over the finished picture (see
//! `crate::LatticeLabels`). Everything below is written for THIS callback —
//! a pass over a picture that has no depth and no order to belong to — and
//! the pieces the lattice reuses are the ones that say `pub(crate)`.
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
        ],
    };
}

/// One ring of the rim: how far out it sits (points), how opaque each stamp
/// is, and how many stamps go round it.
///
/// The look lives in the UI layer, which owns what a label should look like;
/// this crate is handed the numbers. Samples of 0 is a ring that isn't
/// there — which is what the default is, since a caller with no rings to
/// name has no rim to draw.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
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
pub fn text_paint_callback(
    rect: egui::Rect,
    glyphs: Vec<GlyphInstance>,
    rings: [TextRing; 2],
    atlas: Option<FontAtlas>,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
) -> egui::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(
        rect,
        TextCallback { glyphs, rings, atlas, target_format, pane_id },
    )
}

struct TextCallback {
    glyphs: Vec<GlyphInstance>,
    rings: [TextRing; 2],
    atlas: Option<FontAtlas>,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
}

/// What a glyph pipeline is told about the surface it is drawing on.
///
/// `screen_points` is whatever space the glyph rects are quoted in: egui's
/// whole surface for the callback below, one pane for the lattice's own pass
/// (see `crate::LatticeLabels`). `pixels_per_point` is not that space — it is
/// the DEVICE scale the atlas was rasterized at, which is what turns a rim
/// radius in points into a texel offset, and it stays the device's whatever
/// the target's pixels are.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct TextUniforms {
    pub(crate) screen_points: [f32; 2],
    pub(crate) atlas_size: [f32; 2],
    pub(crate) pixels_per_point: f32,
    /// WGSL aligns a `vec4<f32>` to 16 bytes, so the rings start at 32 and
    /// this is the gap in front of them. Named rather than derived because
    /// the mismatch is a validation error at first paint, not a compile one.
    pub(crate) _pad: [f32; 3],
    pub(crate) ring0: [f32; 4],
    pub(crate) ring1: [f32; 4],
}

impl TextUniforms {
    /// The rings as the shader takes them: (radius in points, stamp alpha,
    /// samples, 0).
    pub(crate) fn ring(r: TextRing) -> [f32; 4] {
        [r.radius, r.alpha, r.samples as f32, 0.0]
    }
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
    atlas: MirroredAtlas,
    panes: HashMap<u64, TextPane>,
}

/// Our own copy of egui's font atlas: the texture, its size, and the key of
/// what is in it.
///
/// A paint callback cannot bind the texture egui uploaded — `CallbackResources`
/// holds what WE put there — so the atlas is mirrored. Both renderers that draw
/// glyphs keep one of these: the callback below, and the lattice, which draws
/// its node names inside its own scene pass. Each mirror is the thing an
/// [`MirroredAtlas`]-side key answers for, so a renderer with its own
/// texture needs its own key sequence upstream too — two consumers sharing one
/// publisher would each see half the publications and hold half an atlas.
pub(crate) struct MirroredAtlas {
    texture: Option<wgpu::Texture>,
    size: [u32; 2],
    key: u64,
}

impl Default for MirroredAtlas {
    /// The key starts on something no publication can be: the mirror upstream
    /// counts from zero, and a fresh renderer must not read as one that
    /// already holds the first atlas of the session.
    fn default() -> Self {
        MirroredAtlas { texture: None, size: [0, 0], key: u64::MAX }
    }
}

impl MirroredAtlas {
    /// Whether `atlas` is what this already holds, so the upload can be
    /// skipped.
    pub(crate) fn holds(&self, atlas: &FontAtlas) -> bool {
        self.key == atlas.key
    }

    /// Nothing has ever been uploaded: the first frame arrived without an
    /// atlas, and nothing can be drawn until one does.
    pub(crate) fn is_empty(&self) -> bool {
        self.texture.is_none()
    }

    pub(crate) fn size(&self) -> [u32; 2] {
        self.size
    }

    /// The key of what is held — the caller's own record of which upload a
    /// bind group was built against.
    pub(crate) fn key(&self) -> u64 {
        self.key
    }

    pub(crate) fn view(&self) -> Option<wgpu::TextureView> {
        Some(self.texture.as_ref()?.create_view(&Default::default()))
    }

    /// Take a copy of `atlas`, and say whether the texture was RECREATED —
    /// which is what makes every bind group naming it stale. A same-size
    /// upload writes in place, so bind groups keep pointing at the right
    /// texture and only the pixels move.
    pub(crate) fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas: &FontAtlas,
    ) -> bool {
        let size = [atlas.image.width() as u32, atlas.image.height() as u32];
        let recreated = self.texture.is_none() || self.size != size;
        if recreated {
            self.texture = Some(device.create_texture(&wgpu::TextureDescriptor {
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
            self.size = size;
        }
        let texture = self.texture.as_ref().expect("created above");
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
        self.key = atlas.key;
        recreated
    }
}

struct TextPane {
    uniform_buffer: wgpu::Buffer,
    bind_group: Option<wgpu::BindGroup>,
    instance_buffer: wgpu::Buffer,
    capacity: usize,
    count: u32,
}

/// Starting size of a pane's glyph buffer. A lattice full of labels is a few
/// thousand glyphs; it grows by `next_power_of_two` when a frame overflows.
const INITIAL_GLYPH_CAPACITY: usize = 2048;

/// The bindings every glyph pipeline takes: the surface's uniforms, the
/// mirrored atlas, and its sampler. Shared with the lattice, which draws the
/// same glyphs into its own pass.
pub(crate) fn glyph_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
        ],
    })
}

/// Linear, to match how egui samples the same atlas. At the sizes labels are
/// drawn the glyph lands texel for texel on the framebuffer, so this is an
/// identity for the fill and only does real work for the rim's off-grid taps.
pub(crate) fn glyph_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("text_sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
}

impl TextResources {
    fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let layout = glyph_bind_group_layout(device);
        let rim_pipeline = create_text_pipeline(device, target_format, &layout, "fs_rim", None);
        let fill_pipeline = create_text_pipeline(device, target_format, &layout, "fs_fill", None);
        TextResources {
            rim_pipeline,
            fill_pipeline,
            layout,
            sampler: glyph_sampler(device),
            target_format,
            atlas: MirroredAtlas::default(),
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
        if !self.atlas.upload(device, queue, atlas) {
            return;
        }
        let size = self.atlas.size();
        let view = self.atlas.view().expect("uploaded above");
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
            pane.bind_group =
                Some(bind_group(device, layout, sampler, &view, &pane.uniform_buffer));
        }
    }
}

/// Where [`TextUniforms::atlas_size`] sits, for the partial write above.
/// Taken from the type rather than counted, so reordering the struct cannot
/// leave this pointing at `screen_points`.
const ATLAS_SIZE_OFFSET: wgpu::BufferAddress =
    std::mem::offset_of!(TextUniforms, atlas_size) as wgpu::BufferAddress;

/// One surface's bind group: its own uniforms, and the shared atlas and
/// sampler.
pub(crate) fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    view: &wgpu::TextureView,
    uniforms: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("text_bind_group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    })
}

/// One pass's pipeline: instanced quads blended exactly the way egui blends
/// its own text, so a label composites over the picture identically to the
/// stamped version it replaces.
///
/// `scene_depth` says this pipeline draws in the lattice's scene pass, and
/// carries that pass's depth format. A pipeline has to declare what its pass
/// carries, and that is the whole of what this is for — twice over:
///
///   - the depth attachment, which glyphs neither test nor write; they take
///     their place in the same back-to-front order the nodes are drawn in;
///   - the second COLOUR attachment, which glyphs must not write. That one
///     holds the picture without the labels, and it is what the bloom's
///     bright pass reads, so leaving it unwritten here is the whole of how a
///     name stays out of the bloom.
///
/// The alpha blend is egui's own — `src * (1 - dst.a) + dst`, which is the
/// same arithmetic as premultiplied `over` written from the other side, so a
/// glyph composites into the lattice's offscreen exactly as a node does.
pub(crate) fn create_text_pipeline(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    layout: &wgpu::BindGroupLayout,
    fragment: &str,
    scene_depth: Option<wgpu::TextureFormat>,
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
    let mut targets = vec![Some(wgpu::ColorTargetState {
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
    })];
    // The pass's nodes-only attachment, declared and never written — an empty
    // write mask rather than a `None` target, which wgpu rejects: a pipeline's
    // formats have to match the pass's attachment for attachment, so the way
    // to write nothing is to say so in the mask.
    if scene_depth.is_some() {
        targets.push(Some(wgpu::ColorTargetState {
            format: target_format,
            blend: None,
            write_mask: wgpu::ColorWrites::empty(),
        }));
    }
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
            targets: &targets,
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: scene_depth.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
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
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let recreate = callback_resources
            .get::<TextResources>()
            .is_none_or(|r| r.target_format != self.target_format);
        if recreate {
            callback_resources.insert(TextResources::new(device, self.target_format));
        }
        let resources: &mut TextResources =
            callback_resources.get_mut().expect("inserted above when missing");

        if let Some(atlas) = self.atlas.as_ref().filter(|a| !resources.atlas.holds(a)) {
            resources.mirror_atlas(device, queue, atlas);
        }
        // No atlas yet means the first frame arrived without one: nothing can
        // be drawn, and the next frame that sees a change will bring it.
        if resources.atlas.is_empty() {
            return Vec::new();
        }

        let ppp = screen_descriptor.pixels_per_point.max(f32::EPSILON);
        let atlas_size = resources.atlas.size();
        let uniforms = TextUniforms {
            screen_points: [
                screen_descriptor.size_in_pixels[0] as f32 / ppp,
                screen_descriptor.size_in_pixels[1] as f32 / ppp,
            ],
            atlas_size: [atlas_size[0] as f32, atlas_size[1] as f32],
            pixels_per_point: ppp,
            _pad: [0.0; 3],
            ring0: TextUniforms::ring(self.rings[0]),
            ring1: TextUniforms::ring(self.rings[1]),
        };

        let view = resources.atlas.view().expect("checked above");
        let (layout, sampler) = (&resources.layout, &resources.sampler);
        let pane = resources.panes.entry(self.pane_id).or_insert_with(|| TextPane {
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
        if pane.bind_group.is_none() {
            pane.bind_group =
                Some(bind_group(device, layout, sampler, &view, &pane.uniform_buffer));
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
pub(crate) mod tests {
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
        let _resources = TextResources::new(&device, FORMAT);
    }

    /// A stand-in atlas: one opaque 8x8 "glyph" at (8, 8), with nothing
    /// around it. Coverage is the alpha channel, as egui's atlas stores it.
    ///
    /// Shared with the lattice's own tests, which draw the same glyph through
    /// the same shader in a different pass — so a fixture that changed under
    /// one of them would change under both.
    pub(crate) fn atlas() -> FontAtlas {
        let mut image = egui::ColorImage::filled([32, 32], egui::Color32::TRANSPARENT);
        for y in 8..16 {
            for x in 8..16 {
                image[(x, y)] = egui::Color32::WHITE;
            }
        }
        FontAtlas { image: std::sync::Arc::new(image), key: 1 }
    }

    /// Draw one glyph through both passes and read the frame back.
    fn draw(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        glyph: GlyphInstance,
        rings: [TextRing; 2],
    ) -> Vec<u8> {
        let cb = TextCallback {
            glyphs: vec![glyph],
            rings,
            atlas: Some(atlas()),
            target_format: FORMAT,
            pane_id: 0,
        };
        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(device, queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));

        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32));
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
    pub(crate) fn glyph() -> GlyphInstance {
        GlyphInstance {
            rect: [24.0, 24.0, 8.0, 8.0],
            uv: [8.0, 8.0, 16.0, 16.0],
            fill: [255, 255, 255, 255],
            rim: [255, 0, 0, 255],
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
            glyphs: vec![GlyphInstance { rect: [x, 24.0, 8.0, 8.0], ..glyph() }],
            rings: bare,
            atlas,
            target_format: FORMAT,
            pane_id,
        };

        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32));
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
                glyphs: vec![reaching],
                rings: bare,
                atlas: None,
                target_format: FORMAT,
                pane_id: 0,
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
