//! Shadowed label text, drawn as one instanced quad per glyph through a wgpu
//! paint callback.
//!
//! **Why this exists.** A label used to be stamped around twenty offsets to
//! make a rim. Each glyph now carries fixed near/coarse signed-distance
//! patches. Distance stores that field in its caster cell; Gaussian turns the
//! same field into coverage and runs the common separable blur.
//!
//! **What it does NOT do.** It does not render text. egui still owns the
//! fonts, the shaping, the layout and the atlas — this takes the glyphs it
//! has already placed ([`GlyphInstance`] carries a glyph's screen rect and
//! its rect in egui's own font atlas) and decides only how they reach the
//! framebuffer. Glyph rasterization is untouched, so a label here is the
//! same pixels as the rest of the UI's text.
//!
//! Nor does it rasterize the DRAWN marks — the accidentals and comma signs a
//! note name carries, which the UI cuts and packs into a sheet of its own for
//! the same reasons egui has an atlas. They arrive as instances like any
//! other, naming that sheet in [`GlyphInstance::atlas`], and everything below
//! treats them as glyphs: one quad, the shadow from the fixed SDF, and
//! whatever place in the draw order the run they were collected with has.
//!
//! **Who else draws through it.** The pipelines, atlas binding and the
//! shader are shared with the lattice, which draws its node names inside its
//! own scene pass rather than over the finished picture (see
//! `crate::LatticeLabels`). Everything below is written for THIS callback —
//! a pass over a picture that has no depth and no order to belong to — and
//! the pieces the lattice reuses are the ones that say `pub(crate)`.
//!
//! **Compositing.** Every glyph shadow is laid down before any visible glyph
//! fill. The spectral panes keep their skin-coloured knockout and the lattice
//! keeps its scene-order compositor; only the field producing their coverage
//! is shared.

use std::collections::HashMap;

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};

use crate::{create_vertex_buffer, wgpu};

pub(crate) const TEXT_SRC: &str = include_str!("shaders/text.wgsl");

/// Clear texels carried around a glyph's high-resolution distance patch.
pub const GLYPH_SDF_NEAR_PAD: u32 = 32;
/// Clear texels carried around a glyph's coarse distance patch.
pub const GLYPH_SDF_COARSE_PAD: u32 = 48;
/// Near-field texels over which sampling hands off to the coarse field.
pub const GLYPH_SDF_NEAR_BLEND: u32 = 8;

/// Entry points the text shader must provide.
#[cfg(any(test, feature = "hot-reload"))]
pub(crate) const TEXT_ENTRY_POINTS: &[&str] = &[
    "vs_glyph",
    "vs_spectral_shadow",
    "fs_spectral_shadow",
    "fs_fill",
    "fs_fill_lit",
    "vs_glyph_cell",
    "fs_glyph_ink",
    "vs_glyph_distance_cell",
    "fs_glyph_distance",
    "fs_glyph_sdf_coverage",
    "vs_distance_pad",
    "fs_distance_pad",
    "vs_shadow_box",
    "fs_shadow_box",
];

/// One glyph: where it goes on screen, where it lives in the atlas it is cut
/// from, and the two colors it is drawn in.
///
/// A letter's rects come straight out of the galley egui laid out, so this
/// crate never learns what the text says or which font it is in. A drawn MARK
/// arrives the same way from a rasterizer of the UI's own ([`Self::atlas`]),
/// and everything downstream — the quad, the patch bound, the shadow's
/// arithmetic — treats the two alike.
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
    /// Screen rect the fixed SDF's ink box maps onto: min, then size. The
    /// typeset path is the visible [`Self::rect`]; a drawn mark uses this
    /// second rect so the clear bitmap margin does not become part of its
    /// letterform when the scale-free field is sampled.
    pub sdf_rect: [f32; 4],
    /// The near and coarse SDF texel rectangles corresponding to
    /// [`Self::sdf_rect`], min then max. Zero rectangles mean this glyph has
    /// no distance entry and contributes nothing to a distance cell.
    pub sdf_near: [f32; 4],
    pub sdf_coarse: [f32; 4],
    /// Premultiplied sRGB bytes, straight out of [`egui::Color32`].
    pub fill: [u8; 4],
    /// The shadow's pane-specific color and strength. A fully transparent
    /// value skips this glyph as a caster.
    pub rim: [u8; 4],
    /// Which of the pass's two textures [`Self::uv`] addresses:
    /// [`Self::TYPE`] or [`Self::MARK`].
    ///
    /// Two textures rather than one because egui owns its font atlas while the
    /// UI owns the sheet of drawn marks. This selector lets one instance path
    /// sample either without copying one owner's pixels into the other's.
    pub atlas: u32,
}

impl GlyphInstance {
    /// [`Self::atlas`]: a letter, in egui's font atlas.
    pub const TYPE: u32 = 0;
    /// [`Self::atlas`]: a drawn mark, in the marks' own.
    pub const MARK: u32 = 1;

    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<GlyphInstance>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x4, // rect
            1 => Float32x4, // uv
            2 => Float32x4, // sdf_rect
            3 => Float32x4, // sdf_near
            4 => Float32x4, // sdf_coarse
            5 => Unorm8x4,  // fill
            6 => Unorm8x4,  // pane-specific shadow color
            7 => Uint32,    // atlas
        ],
    };
}

/// A CPU sheet a glyph can be cut from: the drawn marks' atlas, or egui's font
/// atlas in a shell that cannot publish its renderer texture to callbacks.
/// The key changes whenever its pixels do.
pub struct FontAtlas {
    pub image: std::sync::Arc<egui::ColorImage>,
    pub key: u64,
}

/// The process-stable signed-distance sheet used by lattice name shadows.
/// Values are signed distances in atlas texels; each instance carries the
/// mapping that turns them back into pane points.
#[derive(Clone)]
pub struct GlyphSdfAtlas {
    pub image: std::sync::Arc<Vec<f32>>,
    pub size: [u32; 2],
    pub key: u64,
}

/// Draw `glyphs` into `rect`. `pane_id` must be unique per pane drawing text
/// in the same frame (each keeps its own instance buffer; the pipeline and
/// the atlases are shared).
///
/// `atlas` is the fallback for shells that cannot publish egui's renderer
/// texture through `CallbackResources`. `marks` is `None` on frames where the
/// drawn-mark sheet has not changed.
///
/// `slide` is the axis this pane's text scrolls along, which the reconstruction
/// filter follows — see [`SlideAxis`].
/// `pass_nr` is the painter context's cumulative pass number.
#[allow(clippy::too_many_arguments)]
pub fn text_paint_callback(
    rect: egui::Rect,
    glyphs: Vec<GlyphInstance>,
    shadow: Option<harmonigraph_scene::ShadowStyle>,
    atlas: Option<FontAtlas>,
    marks: Option<FontAtlas>,
    sdf: Option<GlyphSdfAtlas>,
    slide: SlideAxis,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
    shadow_surface_id: Option<u64>,
    pass_nr: u64,
) -> egui::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(
        rect,
        TextCallback {
            glyphs,
            shadow,
            atlas,
            marks,
            sdf,
            slide,
            target_format,
            pane_id,
            shadow_surface_id,
            pass_nr,
        },
    )
}

struct TextCallback {
    glyphs: Vec<GlyphInstance>,
    shadow: Option<harmonigraph_scene::ShadowStyle>,
    atlas: Option<FontAtlas>,
    marks: Option<FontAtlas>,
    sdf: Option<GlyphSdfAtlas>,
    slide: SlideAxis,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
    shadow_surface_id: Option<u64>,
    pass_nr: u64,
}

/// What a glyph pipeline is told about the surface it is drawing on.
///
/// `screen_points` is whatever space the glyph rects are quoted in: egui's
/// whole surface for the callback below, one pane for the lattice's own pass
/// (see `crate::LatticeLabels`). `pixels_per_point` is not that space — it is
/// the DEVICE scale the visible atlas was rasterized at, and stays the
/// device's whatever the target's pixels are.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct TextUniforms {
    pub(crate) screen_points: [f32; 2],
    /// The two sheets' sizes in texels, ADJACENT so one partial write covers
    /// the pair — see [`ATLAS_SIZES_OFFSET`], which is the whole reason they
    /// sit together rather than each beside what it belongs to.
    pub(crate) atlas_size: [f32; 2],
    pub(crate) mark_atlas_size: [f32; 2],
    /// The axes this surface's labels slide along — see [`SlideAxis`], and
    /// `FILTER_TAP` in the shader for what is done with them.
    pub(crate) filter_axis: [f32; 2],
    pub(crate) pixels_per_point: f32,
    /// This text group's Shadow depth, 0..=1 — what a name's shadow takes of
    /// whatever stands under it. Lattice text spends it in `fs_shadow_box`;
    /// spectral text spends it in `fs_spectral_shadow`.
    pub(crate) shadow_depth: f32,
    /// The lattice's shadow atlas, in texels — the target `vs_glyph_cell` maps
    /// a name's cell into. 0 everywhere else, where nothing draws into one.
    ///
    /// Here rather than beside the two sheets above because a `vec2` is aligned
    /// to eight bytes and the scalar before it is the pair that reaches it: the
    /// depth and this size sit together so the struct needs no pad of its own.
    pub(crate) shadow_atlas_size: [f32; 2],
    pub(crate) _pad: [f32; 4],
}

/// Which screen axes a surface's labels TRAVEL along, for the taps `coverage`
/// reconstructs a glyph through.
///
/// A closed enum rather than a bare vector, and it is the `Default` that earns
/// it: a zero vector is not neutral but all taps in the same place — the single
/// tap this exists to replace, restored silently and measurable only on a GPU
/// probe. Nothing that derives `Default` can reach that.
///
/// A scrolling surface names one axis and pays two taps. The lattice's camera
/// can carry a label along both at once, so it names both and pays four taps;
/// keeping that choice here prevents every other text surface from paying for
/// motion it cannot make (see `FILTER_TAP`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SlideAxis {
    /// Across the screen — the default, and what every surface takes unless
    /// it names vertical or two-axis motion explicitly.
    #[default]
    Across,
    /// Up and down it: the analyzer with its now-line at the top or bottom,
    /// where time runs down the pane and the names ride it.
    Down,
    /// Along either axis: the lattice, whose orbiting camera moves labels in
    /// both at once and has no single travel direction.
    Both,
}

impl SlideAxis {
    /// `Down` when the surface's text scrolls vertically, `Across` otherwise.
    pub fn vertical(down: bool) -> Self {
        if down {
            Self::Down
        } else {
            Self::Across
        }
    }

    /// The unit vector, as the shader takes it.
    ///
    /// Public because it is the only reading of this type that cannot be
    /// cancelled: a caller checking its own choice against
    /// [`vertical`](Self::vertical) is checking a constructor against itself,
    /// and passes whichever way that constructor maps its argument.
    pub fn unit(self) -> [f32; 2] {
        match self {
            Self::Across => [1.0, 0.0],
            Self::Down => [0.0, 1.0],
            Self::Both => [std::f32::consts::FRAC_1_SQRT_2; 2],
        }
    }
}

struct TextResources {
    /// The SDF shadow composite and the visible glyph fill.
    shadow_pipeline: wgpu::RenderPipeline,
    fill_pipeline: wgpu::RenderPipeline,
    glyph_sdf_coverage_pipeline: wgpu::RenderPipeline,
    glyph_distance_pipeline: wgpu::RenderPipeline,
    glyph_distance_pad_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    shadow_layout: wgpu::BindGroupLayout,
    caster_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    target_format: wgpu::TextureFormat,
    /// Which reload the two pipelines above were built from
    /// (`crate::reload::generation`). Compared exactly as `target_format` is,
    /// and for the same reason: both say the pipelines in hand were built
    /// against something the next frame is not drawing with. The lattice's
    /// reload cannot reach this entry of `CallbackResources` to swap them, so
    /// what it leaves is a count and a source.
    #[cfg(feature = "hot-reload")]
    generation: u64,
    /// The font atlas texture this callback binds.
    atlas: AtlasTexture,
    /// And the drawn marks', which a session that never draws one leaves
    /// empty for its whole life.
    marks: AtlasTexture,
    sdf: SdfTexture,
    blank: wgpu::Texture,
    blank_sdf: wgpu::Texture,
    /// The text callback does not cast lattice shadows, but shares this bind
    /// group layout with the lattice and therefore binds a typed stand-in.
    panes: HashMap<u64, TextPane>,
}

/// One glyph sheet bound by a renderer: either egui's shared GPU texture or a
/// private texture uploaded from a [`FontAtlas`] fallback. The key identifies
/// which texture a pane's bind group names.
pub(crate) struct AtlasTexture {
    texture: Option<wgpu::Texture>,
    size: [u32; 2],
    /// Monotonic identity of the texture allocation that bind groups name.
    binding_key: u64,
    /// The CPU fallback publication currently held in a private texture.
    fallback_key: Option<u64>,
    /// Whether `texture` belongs to egui rather than this binding. A fallback
    /// upload must allocate its own texture even when the dimensions match.
    shared: bool,
}

impl Default for AtlasTexture {
    /// The generation starts at the pane's empty sentinel, so the first
    /// allocation wraps to zero and cannot read as already bound.
    fn default() -> Self {
        AtlasTexture {
            texture: None,
            size: [0, 0],
            binding_key: u64::MAX,
            fallback_key: None,
            shared: false,
        }
    }
}

impl AtlasTexture {
    /// Whether `atlas` is what this already holds, so the upload can be
    /// skipped.
    pub(crate) fn holds(&self, atlas: &FontAtlas) -> bool {
        !self.shared && self.fallback_key == Some(atlas.key)
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
        self.binding_key
    }

    pub(crate) fn view(&self) -> Option<wgpu::TextureView> {
        Some(self.texture.as_ref()?.create_view(&Default::default()))
    }

    /// The bound texture, or `blank` while there is none — the form a bind
    /// group takes, since a binding cannot be left unfilled.
    pub(crate) fn view_or(&self, blank: &wgpu::Texture) -> wgpu::TextureView {
        self.view().unwrap_or_else(|| blank.create_view(&Default::default()))
    }

    /// Bind an existing texture, returning whether bind groups naming the
    /// previous allocation are stale. Equality is GPU-resource identity: an
    /// in-place atlas patch keeps every bind group valid, while a full egui
    /// atlas replacement changes identity even at the same dimensions.
    pub(crate) fn share(&mut self, texture: &wgpu::Texture) -> bool {
        if self.shared && self.texture.as_ref() == Some(texture) {
            return false;
        }
        self.size = [texture.width(), texture.height()];
        self.texture = Some(texture.clone());
        self.binding_key = self.binding_key.wrapping_add(1);
        self.fallback_key = None;
        self.shared = true;
        true
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
        let recreated = self.shared || self.texture.is_none() || self.size != size;
        if recreated {
            self.texture = Some(device.create_texture(&wgpu::TextureDescriptor {
                label: Some("text_font_atlas"),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }));
            self.size = size;
            self.binding_key = self.binding_key.wrapping_add(1);
        }
        self.shared = false;
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
        self.fallback_key = Some(atlas.key);
        recreated
    }
}

/// The lattice renderer's GPU copy of the fixed signed-distance sheet.
/// Unlike egui's font atlas it never grows or moves; the key only keeps a
/// recreated shell from mistaking another allocation for the one it bound.
#[derive(Default)]
pub(crate) struct SdfTexture {
    texture: Option<wgpu::Texture>,
    key: Option<u64>,
}

impl SdfTexture {
    pub(crate) fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas: &GlyphSdfAtlas,
    ) -> bool {
        if self.key == Some(atlas.key) {
            return false;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("text_sdf_atlas"),
            size: wgpu::Extent3d {
                width: atlas.size[0].max(1),
                height: atlas.size[1].max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            bytemuck::cast_slice(atlas.image.as_slice()),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(atlas.size[0] * 4),
                rows_per_image: Some(atlas.size[1]),
            },
            wgpu::Extent3d {
                width: atlas.size[0],
                height: atlas.size[1],
                depth_or_array_layers: 1,
            },
        );
        self.texture = Some(texture);
        self.key = Some(atlas.key);
        true
    }

    pub(crate) fn key(&self) -> u64 {
        self.key.unwrap_or(0)
    }

    pub(crate) fn view_or(&self, blank: &wgpu::Texture) -> wgpu::TextureView {
        self.texture.as_ref().unwrap_or(blank).create_view(&Default::default())
    }
}

struct TextPane {
    uniform_buffer: wgpu::Buffer,
    bind_group: Option<wgpu::BindGroup>,
    instance_buffer: wgpu::Buffer,
    capacity: usize,
    count: u32,
    last_seen_pass: u64,
}

/// A live text pane prepares once per egui pass. This leaves enough slack for a
/// transiently hidden tab without retaining a closed pane's shadow atlas.
const PANE_TTL_PASSES: u64 = 120;

/// Starting size of a pane's glyph buffer. A lattice full of labels is a few
/// thousand glyphs; it grows by `next_power_of_two` when a frame overflows.
const INITIAL_GLYPH_CAPACITY: usize = 2048;

/// The bindings every glyph pipeline takes: the surface's uniforms, the two
/// sampled sheets, and the sampler they share. Shared with the lattice, which
/// draws the same glyphs into its own pass.
pub(crate) fn glyph_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let sheet = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
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
            sheet(1),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            sheet(3),
            wgpu::BindGroupLayoutEntry {
                binding: 4,
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

/// The module every glyph pipeline is cut from.
///
/// Compiled once per surface and lent to each pipeline, rather than once per
/// pipeline: the four are one text.wgsl behind one 228-line common half, and
/// naga parses whatever it is handed every time it is handed it. The lattice
/// side hoists its own the same way (`shader_src`).
pub(crate) fn glyph_shader(device: &wgpu::Device, source: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("text_shader"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

/// The three draws that prepare a name's shadow cell.
///
/// Under the Gaussian a cell keeps the coverage union it already uses. Under
/// Distance the pass first fills every analytic cell with its own pad, then
/// MIN-blends each glyph's true signed field over that value. The latter is the
/// exact union of the letterforms and cannot double-darken where two glyph
/// quads overlap.
pub(crate) fn create_glyph_cell_pipelines(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline, wgpu::RenderPipeline) {
    const MAX_COMPONENT: wgpu::BlendComponent = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Max,
    };
    const MIN_COMPONENT: wgpu::BlendComponent = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Min,
    };
    let coverage = glyph_pipeline(
        device,
        shader,
        "glyph_coverage_cell",
        &[Some(layout)],
        ("vs_glyph_cell", "fs_glyph_ink"),
        &[GlyphInstance::LAYOUT, crate::shadow::ShadowBox::BESIDE_GLYPHS],
        &[Some(wgpu::ColorTargetState {
            format: crate::shadow::ATLAS_FORMAT,
            blend: Some(wgpu::BlendState { color: MAX_COMPONENT, alpha: MAX_COMPONENT }),
            write_mask: wgpu::ColorWrites::ALL,
        })],
        None,
    );
    let distance = glyph_pipeline(
        device,
        shader,
        "glyph_distance_cell",
        &[Some(layout)],
        ("vs_glyph_distance_cell", "fs_glyph_distance"),
        &[GlyphInstance::LAYOUT, crate::shadow::ShadowBox::BESIDE_GLYPHS],
        &[Some(wgpu::ColorTargetState {
            format: crate::shadow::ATLAS_FORMAT,
            blend: Some(wgpu::BlendState { color: MIN_COMPONENT, alpha: MIN_COMPONENT }),
            write_mask: wgpu::ColorWrites::ALL,
        })],
        None,
    );
    let pad = glyph_pipeline(
        device,
        shader,
        "glyph_distance_cell_pad",
        &[Some(layout)],
        ("vs_distance_pad", "fs_distance_pad"),
        &[crate::shadow::ShadowBox::LAYOUT],
        &[Some(wgpu::ColorTargetState {
            format: crate::shadow::ATLAS_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })],
        None,
    );
    (coverage, distance, pad)
}

/// The spectral text group's Gaussian producer. Unlike the lattice's retained
/// coverage control, this rasterizes coverage from the fixed glyph SDF, so the
/// two spectral kernels share one zero contour.
fn create_glyph_sdf_coverage_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    const MAX_COMPONENT: wgpu::BlendComponent = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Max,
    };
    glyph_pipeline(
        device,
        shader,
        "glyph_sdf_coverage_cell",
        &[Some(layout)],
        ("vs_glyph_distance_cell", "fs_glyph_sdf_coverage"),
        &[GlyphInstance::LAYOUT, crate::shadow::ShadowBox::BESIDE_GLYPHS],
        &[Some(wgpu::ColorTargetState {
            format: crate::shadow::ATLAS_FORMAT,
            blend: Some(wgpu::BlendState { color: MAX_COMPONENT, alpha: MAX_COMPONENT }),
            write_mask: wgpu::ColorWrites::ALL,
        })],
        None,
    )
}

fn create_spectral_shadow_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    atlas: &wgpu::BindGroupLayout,
    casters: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    glyph_pipeline(
        device,
        shader,
        "spectral_text_shadow",
        &[Some(layout), None, Some(atlas), Some(casters)],
        ("vs_spectral_shadow", "fs_spectral_shadow"),
        &[GlyphInstance::LAYOUT, crate::shadow::ShadowBox::BESIDE_GLYPHS],
        &[Some(wgpu::ColorTargetState {
            format: target_format,
            blend: Some(crate::EGUI_BLEND),
            write_mask: wgpu::ColorWrites::ALL,
        })],
        None,
    )
}

/// A name's shadow into the scene pass, over the name's own box
/// (`fs_shadow_box`).
///
/// BOTH of the pass's attachments, under the premultiplied blend the multiply
/// rides on: the bloom reads the second, and a halo a name darkens has to bloom
/// as darkened. The glyphs beside it write the first alone
/// ([`create_text_pipeline`]), which is what keeps the name itself out of the
/// bloom.
///
/// Four groups, the second empty: the pane's uniforms, the atlas at group 2 and
/// the casters' kernels at group 3, where the shader declares each — the light
/// at group 1 is the fill's and this draw reads none.
///
/// NO VERTEX BUFFER. The draw is one instance at the caster's own index, and
/// every number the quad and the mix need is in the array at group 3
/// (`vs_shadow_box`).
pub(crate) fn create_shadow_box_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    atlas: &wgpu::BindGroupLayout,
    casters: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    depth: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let target = Some(wgpu::ColorTargetState {
        format: target_format,
        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        write_mask: wgpu::ColorWrites::ALL,
    });
    glyph_pipeline(
        device,
        shader,
        "fs_shadow_box",
        &[Some(layout), None, Some(atlas), Some(casters)],
        ("vs_shadow_box", "fs_shadow_box"),
        &[],
        &[target.clone(), target],
        Some(depth),
    )
}

/// The shape every pipeline above shares: one shader module, a triangle strip,
/// no multisampling, and a depth state only where the pass carries one.
///
/// Spelled once because the four differ in nothing else, and the fields that
/// look like they might — the primitive topology, the vertex step mode — are
/// properties of how this shader draws rather than of any one entry point.
///
/// `shader` is [`glyph_shader`]'s, handed in rather than built here: a caller
/// builds several of these at once off one module, and a hot-reload's module is
/// not the baked one.
#[allow(clippy::too_many_arguments)]
fn glyph_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    label: &str,
    groups: &[Option<&wgpu::BindGroupLayout>],
    entries: (&str, &str),
    buffers: &[wgpu::VertexBufferLayout<'static>],
    targets: &[Option<wgpu::ColorTargetState>],
    depth: Option<wgpu::TextureFormat>,
) -> wgpu::RenderPipeline {
    let (vertex, fragment) = entries;
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("text_pipeline_layout"),
        bind_group_layouts: groups,
        ..Default::default()
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
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
            targets,
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: depth.map(|format| wgpu::DepthStencilState {
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

/// Linear, to match how egui samples the same atlas — and it does real work on
/// every tap of the visible fill.
///
/// Which is worth stating, because the reverse is the plausible reading and it
/// is wrong in both of the ways a glyph reaches the framebuffer. A label is
/// placed where it is handed rather than rounded onto a whole physical pixel
/// (`harmonigraph_ui::text::TextBatch::text`, which explains why), and the size
/// ladder draws it a percent or two off the size its atlas patch was
/// rasterized at. A glyph landing texel for texel is the rare case, not the
/// ordinary one, so nothing about the fill path is exempt from what this
/// sampler does — including its `ClampToEdge`, which is what `text.wgsl`'s
/// `outside_atlas` exists to answer for.
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
    /// Whether the pipelines in hand were built for something the next frame is
    /// not drawing: a different surface format, or — under hot-reload — a build
    /// of the text module older than the one last published.
    ///
    /// Two questions with one answer because the remedy is one: these pipelines
    /// are rebuilt from scratch either way, there being nothing in a
    /// `RenderPipeline` to patch.
    fn is_stale(&self, target_format: wgpu::TextureFormat) -> bool {
        #[cfg(feature = "hot-reload")]
        if self.generation != crate::reload::generation() {
            return true;
        }
        self.target_format != target_format
    }

    /// The spectral shadow, its two SDF producers, and the fill pass off one
    /// read of the module. Split out
    /// because a reload owes exactly this pair and nothing else in the struct
    /// — see [`TextResources::rebuild_pipelines`].
    fn pipelines(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        shadow_layout: &wgpu::BindGroupLayout,
        caster_layout: &wgpu::BindGroupLayout,
        target_format: wgpu::TextureFormat,
    ) -> (
        wgpu::RenderPipeline,
        wgpu::RenderPipeline,
        wgpu::RenderPipeline,
        wgpu::RenderPipeline,
        wgpu::RenderPipeline,
    ) {
        let shader = glyph_shader(device, &crate::text_source());
        let shadow_pipeline = create_spectral_shadow_pipeline(
            device,
            &shader,
            layout,
            shadow_layout,
            caster_layout,
            target_format,
        );
        let fill_pipeline = create_text_pipeline(
            device,
            &shader,
            target_format,
            layout,
            None,
            ("vs_glyph", "fs_fill"),
            None,
            crate::EGUI_BLEND,
        );
        let (_, distance, pad) = create_glyph_cell_pipelines(device, &shader, layout);
        let coverage = create_glyph_sdf_coverage_pipeline(device, &shader, layout);
        (shadow_pipeline, fill_pipeline, coverage, distance, pad)
    }

    /// Swap in pipelines built for what the next frame is drawing, keeping the
    /// atlas bindings and every pane prepared against them.
    ///
    /// Nothing carried over depends on the module or the target format: the
    /// atlas and the marks are SAMPLED textures and a pane's bind group is
    /// built against the layout this keeps, so replacing the pair below is the
    /// whole of the debt.
    fn rebuild_pipelines(&mut self, device: &wgpu::Device, target_format: wgpu::TextureFormat) {
        // The source is read inside, and BEFORE the count below: a reload
        // committed between the two would raise a count this build has not
        // taken, and the rebuild it is owed would then never be asked for.
        let (shadow_pipeline, fill_pipeline, coverage, distance, pad) = Self::pipelines(
            device,
            &self.layout,
            &self.shadow_layout,
            &self.caster_layout,
            target_format,
        );
        #[cfg(feature = "hot-reload")]
        {
            self.generation = crate::reload::generation();
        }
        self.shadow_pipeline = shadow_pipeline;
        self.fill_pipeline = fill_pipeline;
        self.glyph_sdf_coverage_pipeline = coverage;
        self.glyph_distance_pipeline = distance;
        self.glyph_distance_pad_pipeline = pad;
        self.target_format = target_format;
    }

    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        shadow_layouts: &crate::spectral_shadow::Layouts,
    ) -> Self {
        let layout = glyph_bind_group_layout(device);
        let shadow_layout = shadow_layouts.atlas.clone();
        let caster_layout = shadow_layouts.casters.clone();
        // Read once for the pair, and BEFORE the count below: a reload
        // committed between the two would raise a count this build has not
        // taken, and the rebuild it is owed would then never be asked for.
        let (shadow_pipeline, fill_pipeline, coverage, distance, pad) =
            Self::pipelines(device, &layout, &shadow_layout, &caster_layout, target_format);
        #[cfg(feature = "hot-reload")]
        let generation = crate::reload::generation();
        TextResources {
            shadow_pipeline,
            fill_pipeline,
            glyph_sdf_coverage_pipeline: coverage,
            glyph_distance_pipeline: distance,
            glyph_distance_pad_pipeline: pad,
            layout,
            shadow_layout,
            caster_layout,
            sampler: glyph_sampler(device),
            target_format,
            #[cfg(feature = "hot-reload")]
            generation,
            atlas: AtlasTexture::default(),
            marks: AtlasTexture::default(),
            sdf: SdfTexture::default(),
            blank: blank_atlas(device, queue),
            blank_sdf: blank_sdf_atlas(device, queue),
            panes: HashMap::new(),
        }
    }

    /// Bind the current font and mark sheets, carrying every pane already
    /// prepared this frame onto any replacement texture.
    ///
    /// That last part is the whole of why this is not four lines. egui-wgpu
    /// runs EVERY callback's `prepare` and only then every `paint`. The shared
    /// font texture is final before that sequence begins, while the mark sheet
    /// is carried by whichever pane first packed a new mark. Every pane that
    /// prepared before a replacement already had its turn:
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
    /// nothing. The font texture's resource identity changes whenever egui
    /// replaces it, including a same-size rebuild, so its bind groups cannot
    /// silently point at the allocation before the rebuild.
    ///
    /// The mark atlas owes the same two things and pays them the same way:
    /// `harmonigraph_ui::text::MarkAtlas` hands out a patch before deciding
    /// whether to publish, and it only ever APPENDS within a pass — a repack
    /// waits for the next one, where it is ahead of every uv rather than
    /// behind some of them.
    fn bind_sheets(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shared_atlas: Option<&wgpu::Texture>,
        fallback_atlas: Option<&FontAtlas>,
        marks: Option<&FontAtlas>,
        sdf: Option<&GlyphSdfAtlas>,
    ) {
        let mut recreated = false;
        if let Some(atlas) = fallback_atlas.filter(|a| !self.atlas.holds(a)) {
            recreated |= self.atlas.upload(device, queue, atlas);
        } else if let Some(atlas) = shared_atlas {
            recreated |= self.atlas.share(atlas);
        }
        if let Some(marks) = marks.filter(|a| !self.marks.holds(a)) {
            recreated |= self.marks.upload(device, queue, marks);
        }
        if let Some(sdf) = sdf {
            recreated |= self.sdf.upload(device, queue, sdf);
        }
        if !recreated {
            return;
        }
        let sizes = self.atlas_sizes();
        let view = self.atlas.view_or(&self.blank);
        let mark_view = self.marks.view_or(&self.blank);
        let (layout, sampler) = (&self.layout, &self.sampler);
        let sdf_view = self.sdf.view_or(&self.blank_sdf);
        for pane in self.panes.values_mut() {
            // The sizes alone, not the whole struct: a pane that has already
            // prepared wrote the rest of its uniforms this frame, and one that
            // has not is about to write all of them including these.
            queue.write_buffer(
                &pane.uniform_buffer,
                ATLAS_SIZES_OFFSET,
                bytemuck::cast_slice(&sizes),
            );
            pane.bind_group = Some(bind_group(
                device,
                layout,
                sampler,
                &view,
                &mark_view,
                &sdf_view,
                &pane.uniform_buffer,
            ));
        }
    }

    /// The two sheets' sizes, as the uniforms carry them: font atlas then
    /// marks, which is the order the struct puts them in.
    ///
    /// A sheet that has never been uploaded reports the 1x1 blank standing in
    /// for it rather than its own zero, because the shader DIVIDES by this: a
    /// zero there is a NaN coverage, and NaN fails the `<= 0` that would have
    /// discarded it, so the one glyph that reached an empty sheet would paint
    /// garbage instead of nothing.
    fn atlas_sizes(&self) -> [f32; 4] {
        let (a, m) = (self.atlas.size(), self.marks.size());
        [a[0], a[1], m[0], m[1]].map(|n| n.max(1) as f32)
    }
}

/// Where [`TextUniforms::atlas_size`] sits, for the partial write above —
/// which covers the mark atlas's size too, the two being adjacent. Taken from
/// the type rather than counted, so reordering the struct cannot leave this
/// pointing at `screen_points`.
const ATLAS_SIZES_OFFSET: wgpu::BufferAddress =
    std::mem::offset_of!(TextUniforms, atlas_size) as wgpu::BufferAddress;

/// One surface's bind group: its own uniforms, the two shared sheets, and the
/// sampler they share.
pub(crate) fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    view: &wgpu::TextureView,
    mark_view: &wgpu::TextureView,
    sdf_view: &wgpu::TextureView,
    uniforms: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("text_bind_group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(mark_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(sdf_view),
            },
        ],
    })
}

/// A 1x1 transparent texture, bound wherever a sheet has not arrived.
///
/// The bind group layout fixes both bindings, so both have to name a texture
/// from the first frame — and a sheet can be missing for a whole session
/// rather than for a moment: egui's atlas arrives only once some pane has laid
/// text out, and the mark atlas is never built at all in a shell that draws no
/// note names. Nothing samples this. A glyph pointing at a sheet is one the
/// batch cut FROM that sheet, and its callback receives that sheet before it
/// prepares the draw.
pub(crate) fn blank_atlas(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("text_blank_atlas"),
        size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    // Written rather than left as allocated: a texture's initial contents are
    // undefined, and "nothing samples it" is an argument about this code
    // rather than a promise the driver makes.
    queue.write_texture(
        texture.as_image_copy(),
        &[0u8; 4],
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
        wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
    );
    texture
}

/// A typed stand-in for the fixed R32Float signed-distance sheet.
pub(crate) fn blank_sdf_atlas(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("text_blank_sdf_atlas"),
        size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        bytemuck::bytes_of(&0.0f32),
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
        wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
    );
    texture
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
/// `blend` is [`crate::EGUI_BLEND`]. Its alpha term is egui's own —
/// `src * (1 - dst.a) + dst`, which is the same arithmetic as premultiplied
/// `over` written from the other side, so a glyph composites into the lattice's
/// offscreen exactly as a node does.
///
/// `entries` is this pipeline's two entry points, vertex then fragment, and
/// they are handed over as a PAIR because they are chosen together: what a
/// fragment entry paints outside a glyph's ink is what its vertex entry has to
/// grow the quad by, and a fragment reaching past its own quad is a shape cut
/// off in a screen-aligned line rather than a compile error.
///
/// `glow` is the lattice's light, at group 1, and only `fs_fill_lit` reads it:
/// a name there is ink standing in the light and takes the wash a marker's
/// cross takes. Every other surface passes `None` — its text has no light to
/// stand in, and the entry points it draws through name no group 1 for a layout
/// to have to carry. The shadow a name casts is the other half of the same
/// picture and is a draw of its own; see [`create_shadow_box_pipeline`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_text_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    target_format: wgpu::TextureFormat,
    layout: &wgpu::BindGroupLayout,
    glow: Option<&wgpu::BindGroupLayout>,
    entries: (&str, &str),
    scene_depth: Option<wgpu::TextureFormat>,
    blend: wgpu::BlendState,
) -> wgpu::RenderPipeline {
    let mut targets = vec![Some(wgpu::ColorTargetState {
        format: target_format,
        blend: Some(blend),
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
    glyph_pipeline(
        device,
        shader,
        entries.1,
        &[Some(layout), glow],
        entries,
        &[GlyphInstance::LAYOUT],
        &targets,
        scene_depth,
    )
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
        let shadow_layouts = crate::spectral_shadow::layouts(device, callback_resources);
        // The shell inserts the current egui font texture under its concrete
        // wgpu type. Cloning this handle does not copy texture data.
        let shared_atlas = callback_resources.get::<wgpu::Texture>().cloned();
        match callback_resources.get_mut::<TextResources>() {
            // Stale pipelines are the whole of what a reload or a format change
            // owes; the atlas binding beside them updates independently, so
            // it survives the pipeline rebuild.
            Some(resources) if resources.is_stale(self.target_format) => {
                resources.rebuild_pipelines(device, self.target_format);
            }
            Some(_) => {}
            None => {
                callback_resources.insert(TextResources::new(
                    device,
                    queue,
                    self.target_format,
                    &shadow_layouts,
                ));
            }
        }
        let resources: &mut TextResources =
            callback_resources.get_mut().expect("inserted above when missing");
        resources
            .panes
            .retain(|_, pane| self.pass_nr.saturating_sub(pane.last_seen_pass) < PANE_TTL_PASSES);

        resources.bind_sheets(
            device,
            queue,
            shared_atlas.as_ref(),
            self.atlas.as_ref(),
            self.marks.as_ref(),
            self.sdf.as_ref(),
        );
        // No atlas yet means the first frame arrived without one: nothing can
        // be drawn, and the next frame that sees a change will bring it.
        if resources.atlas.is_empty() {
            return Vec::new();
        }

        let ppp = screen_descriptor.pixels_per_point.max(f32::EPSILON);
        let style = self.shadow.map(harmonigraph_scene::ShadowStyle::clamped);
        let sigma =
            style.filter(|style| style.casts()).map_or(0.0, crate::shadow::spectral_sigma_points);
        let kernel = style.map_or(harmonigraph_scene::ShadowKernel::Distance, |s| s.kernel);
        let casters: Vec<crate::shadow::Caster> = self
            .glyphs
            .iter()
            .map(|glyph| crate::shadow::Caster {
                rect: glyph.sdf_rect,
                level: f32::from(glyph.rim[3] > 0),
                sigma_points: sigma,
                kernel,
                direct_distance: false,
            })
            .collect();
        let sizes = resources.atlas_sizes();
        let uniforms = TextUniforms {
            screen_points: [
                screen_descriptor.size_in_pixels[0] as f32 / ppp,
                screen_descriptor.size_in_pixels[1] as f32 / ppp,
            ],
            atlas_size: [sizes[0], sizes[1]],
            mark_atlas_size: [sizes[2], sizes[3]],
            filter_axis: self.slide.unit(),
            pixels_per_point: ppp,
            shadow_depth: style.map_or(0.0, |s| s.depth),
            shadow_atlas_size: [1.0; 2],
            _pad: [0.0; 4],
        };

        let view = resources.atlas.view().expect("checked above");
        let mark_view = resources.marks.view_or(&resources.blank);
        let sdf_view = resources.sdf.view_or(&resources.blank_sdf);
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
            last_seen_pass: self.pass_nr,
        });
        pane.last_seen_pass = self.pass_nr;
        if pane.bind_group.is_none() {
            pane.bind_group = Some(bind_group(
                device,
                layout,
                sampler,
                &view,
                &mark_view,
                &sdf_view,
                &pane.uniform_buffer,
            ));
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
        if let Some(surface_id) = self.shadow_surface_id {
            let submission = crate::spectral_shadow::Submission {
                key: crate::spectral_shadow::ProducerKey::Text(self.pane_id),
                casters,
                draw: crate::spectral_shadow::CellDraw::Text {
                    coverage: resources.glyph_sdf_coverage_pipeline.clone(),
                    distance: resources.glyph_distance_pipeline.clone(),
                    distance_pad: resources.glyph_distance_pad_pipeline.clone(),
                    locals: pane.bind_group.as_ref().expect("bound above").clone(),
                    glyphs: pane.instance_buffer.clone(),
                    count: pane.count,
                    kernel,
                },
                atlas_uniform: pane.uniform_buffer.clone(),
                atlas_size_offset: std::mem::offset_of!(TextUniforms, shadow_atlas_size) as u64,
            };
            crate::spectral_shadow::register_for_pass(
                device,
                callback_resources,
                surface_id,
                submission,
                self.pass_nr,
            );
        }

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
        // Every knockout, then every fill: neighbouring glyphs remain one
        // label rather than each shadowing the other's visible ink.
        let shadow = self.shadow_surface_id.and_then(|surface_id| {
            crate::spectral_shadow::binding(
                callback_resources,
                surface_id,
                crate::spectral_shadow::ProducerKey::Text(self.pane_id),
            )
        });
        if let Some(shadow) = shadow.filter(|binding| binding.active) {
            render_pass.set_pipeline(&resources.shadow_pipeline);
            render_pass.set_bind_group(2, shadow.atlas, &[]);
            render_pass.set_bind_group(3, shadow.casters, &[]);
            let stride = std::mem::size_of::<crate::shadow::ShadowBox>() as u64;
            render_pass.set_vertex_buffer(
                1,
                shadow.boxes.slice(
                    stride * u64::from(shadow.start)
                        ..stride * u64::from(shadow.start + shadow.count),
                ),
            );
            render_pass.draw(0..4, 0..pane.count);
        }
        render_pass.set_pipeline(&resources.fill_pipeline);
        render_pass.draw(0..4, 0..pane.count);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The shader expands the ink rectangles back to the packed tile bounds,
    /// so those constants must move with the generator's public contract.
    #[test]
    fn the_sdf_tile_padding_agrees_with_the_shader() {
        let near: f32 =
            crate::shadow::tests::shader_const(TEXT_SRC, "SDF_NEAR_PAD").parse().expect("a number");
        let coarse: f32 = crate::shadow::tests::shader_const(TEXT_SRC, "SDF_COARSE_PAD")
            .parse()
            .expect("a number");
        let blend: f32 = crate::shadow::tests::shader_const(TEXT_SRC, "SDF_NEAR_BLEND")
            .parse()
            .expect("a number");
        assert_eq!(near, GLYPH_SDF_NEAR_PAD as f32);
        assert_eq!(coarse, GLYPH_SDF_COARSE_PAD as f32);
        assert_eq!(blend, GLYPH_SDF_NEAR_BLEND as f32);
    }
    use crate::gpu_harness::{headless_device, readback, render_to_texture};

    const SIZE: [u32; 2] = [64, 64];
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

    #[test]
    fn baked_text_shader_validates() {
        let seam = crate::common_lines(crate::COMMON_SRC);
        crate::validate_wgsl("text.wgsl", &crate::with_common(TEXT_SRC), seam, TEXT_ENTRY_POINTS)
            .expect("baked text.wgsl must parse, validate, and keep its entry points");
    }

    /// The one place a plugin build's text pipelines could break is the source
    /// they compile, so the two ways of naming it have to agree: no hot-reload
    /// feature means `text_source` IS the baked concatenation.
    #[test]
    #[cfg(not(feature = "hot-reload"))]
    fn a_build_with_no_watcher_compiles_the_baked_text_module() {
        assert_eq!(crate::text_source(), crate::with_common(TEXT_SRC));
    }

    #[test]
    fn every_glyph_pipeline_is_built_from_one_module() {
        let src = crate::text_source();
        for required in TEXT_ENTRY_POINTS {
            assert!(
                src.contains(&format!("fn {required}(")),
                "the module every glyph pipeline compiles is missing `{required}`"
            );
        }
    }

    #[test]
    fn either_spectral_text_endpoint_allocates_no_shadow_atlas() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        for style in [
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
            let cb = TextCallback {
                glyphs: vec![glyph()],
                shadow: Some(style),
                atlas: Some(atlas()),
                marks: None,
                sdf: Some(sdf_atlas()),
                slide: SlideAxis::default(),
                target_format: FORMAT,
                pane_id: 0,
                shadow_surface_id: Some(0),
                pass_nr: 0,
            };
            let mut resources = CallbackResources::default();
            let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
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
                "{style:?} allocated a shadow atlas"
            );
        }
    }

    /// A format the surface is not in rebuilds, as it always has. Named here
    /// because it is now one arm of a two-arm test and the other arm must not
    /// be what makes this one pass.
    #[test]
    fn pipelines_built_for_another_format_are_stale() {
        let _guard = crate::reload::test_lock();
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let mut callbacks = CallbackResources::default();
        let layouts = crate::spectral_shadow::layouts(&device, &mut callbacks);
        let resources = TextResources::new(&device, &queue, FORMAT, &layouts);
        assert!(!resources.is_stale(FORMAT));
        assert!(resources.is_stale(wgpu::TextureFormat::Bgra8Unorm));
    }

    /// And a reload published after they were built, which is #510: the
    /// lattice's reload cannot reach this entry of `CallbackResources`, so a
    /// count is what tells the next `prepare` to rebuild. Without it an edit to
    /// common.wgsl leaves every glyph here on the previous build's arithmetic.
    #[test]
    #[cfg(feature = "hot-reload")]
    fn pipelines_built_before_a_reload_are_stale() {
        let _guard = crate::reload::test_lock();
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let mut callbacks = CallbackResources::default();
        let layouts = crate::spectral_shadow::layouts(&device, &mut callbacks);
        let resources = TextResources::new(&device, &queue, FORMAT, &layouts);
        assert!(!resources.is_stale(FORMAT), "nothing has been published since these were built");

        crate::reload::publish(
            format!(
                "{}\n// pipelines_built_before_a_reload_are_stale\n",
                crate::with_common(TEXT_SRC)
            ),
            crate::COMMON_SRC.to_owned(),
        );
        assert!(resources.is_stale(FORMAT), "a reload the text pipelines never hear about");

        // ...and a build taken after it is current again, so the rebuild
        // happens once rather than on every frame from here on.
        let after = TextResources::new(&device, &queue, FORMAT, &layouts);
        assert!(!after.is_stale(FORMAT));
    }

    /// And the rebuild it asks for keeps the atlas binding.
    ///
    /// `pipelines_built_before_a_reload_are_stale` asks only that the rebuild
    /// happen; this asks what it costs. A reload changes shader pipelines, not
    /// the sampled texture or the pane bind groups naming it, so discarding
    /// either would make the next callback wait for unrelated atlas activity
    /// before its labels can return.
    #[test]
    #[cfg(feature = "hot-reload")]
    fn a_reload_rebuilds_the_pipelines_without_dropping_the_atlas() {
        let _guard = crate::reload::test_lock();
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let mut callbacks = CallbackResources::default();
        let layouts = crate::spectral_shadow::layouts(&device, &mut callbacks);
        let mut resources = TextResources::new(&device, &queue, FORMAT, &layouts);

        // A sheet in hand, the way a frame that has drawn one label leaves it.
        resources.bind_sheets(&device, &queue, None, Some(&atlas()), Some(&mark_sheet()), None);
        assert!(
            !resources.atlas.is_empty() && !resources.marks.is_empty(),
            "the fixture never filled the atlas bindings, so what follows measures nothing",
        );

        crate::reload::publish(
            format!(
                "{}\n// a_reload_rebuilds_the_pipelines_without_dropping_the_atlas\n",
                crate::with_common(TEXT_SRC)
            ),
            crate::COMMON_SRC.to_owned(),
        );
        assert!(resources.is_stale(FORMAT), "a reload the text pipelines never heard about");

        resources.rebuild_pipelines(&device, FORMAT);

        assert!(!resources.is_stale(FORMAT), "the rebuild did not take the published build");
        assert!(
            !resources.atlas.is_empty(),
            "the reload dropped the font atlas binding: nothing upstream will send \
             another, so every haloed label stays absent from here on",
        );
        assert!(!resources.marks.is_empty(), "the reload dropped the mark sheet binding");
    }

    #[test]
    fn the_pipelines_build_against_a_headless_device() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let mut callbacks = CallbackResources::default();
        let layouts = crate::spectral_shadow::layouts(&device, &mut callbacks);
        let _resources = TextResources::new(&device, &queue, FORMAT, &layouts);
    }

    /// The shell's font texture is keyed by GPU-resource identity. A patch to
    /// the same allocation keeps pane bind groups valid; a full rebuild at the
    /// same dimensions still replaces the allocation and must rebind them. A
    /// fallback source also owns its allocation even when its publication key
    /// collides with the shared binding's generation.
    #[test]
    fn a_callback_tracks_the_shared_font_texture_by_resource_identity() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let sheet = atlas();
        let mut uploaded = AtlasTexture::default();
        uploaded.upload(&device, &queue, &sheet);
        let shared = uploaded.texture.expect("the fixture uploads a texture");

        let cb = TextCallback {
            glyphs: vec![glyph()],
            shadow: None,
            atlas: None,
            marks: None,
            sdf: None,
            slide: SlideAxis::default(),
            target_format: FORMAT,
            pane_id: 0,
            shadow_surface_id: None,
            pass_nr: 0,
        };
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        let mut resources = CallbackResources::default();
        resources.insert(shared.clone());
        let mut encoder = device.create_command_encoder(&Default::default());
        cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);

        let bound = resources.get::<TextResources>().expect("the callback prepares resources");
        assert_eq!(bound.atlas.texture.as_ref(), Some(&shared));
        assert_eq!(bound.atlas.size(), [sheet.image.width() as u32, sheet.image.height() as u32]);
        let first_key = bound.atlas.key();

        let mut encoder = device.create_command_encoder(&Default::default());
        cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        assert_eq!(
            resources.get::<TextResources>().unwrap().atlas.key(),
            first_key,
            "an in-place atlas patch keeps the same binding key",
        );

        let replacement = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("same_size_rebuilt_font_atlas"),
            size: shared.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        resources.insert(replacement.clone());
        let mut encoder = device.create_command_encoder(&Default::default());
        cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        let rebound = resources.get::<TextResources>().unwrap();
        assert_eq!(rebound.atlas.texture.as_ref(), Some(&replacement));
        assert_ne!(rebound.atlas.key(), first_key, "a new allocation must rebuild bind groups");
        let replacement_key = rebound.atlas.key();

        resources.get_mut::<TextResources>().unwrap().bind_sheets(
            &device,
            &queue,
            None,
            Some(&sheet),
            None,
            None,
        );
        let fallback = resources.get::<TextResources>().unwrap();
        assert!(!fallback.atlas.shared, "the CPU fallback must own its texture");
        assert_ne!(
            fallback.atlas.texture.as_ref(),
            Some(&replacement),
            "the fallback wrote into egui's shared texture",
        );
        assert_ne!(
            fallback.atlas.key(),
            replacement_key,
            "switching sources kept the binding generation despite replacing the allocation",
        );
        assert!(fallback.atlas.holds(&sheet), "the fallback publication was not recorded");
    }

    /// A stand-in atlas: one opaque 8x8 "glyph" at (8, 8), with nothing
    /// around it. Coverage is the alpha channel, as egui's atlas stores it.
    ///
    /// Shared with the lattice's own tests, which draw the same glyph through
    /// the same shader in a different pass — so a fixture that changed under
    /// one of them would change under both.
    pub(crate) fn atlas() -> FontAtlas {
        patch_at([8, 8], 32, 1)
    }

    /// And a stand-in MARK sheet, with its patch somewhere the font atlas has
    /// nothing. That is the whole design of the pair: a glyph reading the
    /// wrong sheet finds transparency and paints nothing, rather than finding
    /// a plausible square of ink and passing.
    ///
    /// TWICE the font atlas's width, which is the other half of the pair. An
    /// instance carries the sheet it reads and the size to measure it by, and
    /// two sheets of one size hold the second constant while the first varies
    /// — so a shader that read the right texture through the font atlas's
    /// dimensions would land on the patch anyway and pass. In the app the two
    /// are never alike (epaint's atlas is 2048 wide against
    /// `MARK_SHEET_WIDTH`), and at 64 the mark's uv normalizes to half of what
    /// the wrong size gives, which is transparency.
    pub(crate) fn mark_sheet() -> FontAtlas {
        patch_at([16, 0], 64, 1)
    }

    /// A `width`x32 sheet with one opaque 8x8 patch at `at`.
    fn patch_at([left, top]: [usize; 2], width: usize, key: u64) -> FontAtlas {
        let mut image = egui::ColorImage::filled([width, 32], egui::Color32::TRANSPARENT);
        for y in top..top + 8 {
            for x in left..left + 8 {
                image[(x, y)] = egui::Color32::WHITE;
            }
        }
        FontAtlas { image: std::sync::Arc::new(image), key }
    }

    /// The mark of [`mark_sheet`], drawn 8 points wide at (24, 24) — the same
    /// place and size as [`glyph`], off the other sheet.
    pub(crate) fn mark() -> GlyphInstance {
        GlyphInstance { uv: [16.0, 0.0, 24.0, 8.0], atlas: GlyphInstance::MARK, ..glyph() }
    }

    /// Draw one glyph through both passes and read the frame back.
    fn draw(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        glyph: GlyphInstance,
        shadow: Option<harmonigraph_scene::ShadowStyle>,
    ) -> Vec<u8> {
        draw_from(device, queue, glyph, shadow, atlas(), SlideAxis::default())
    }

    /// The same, off a sheet the caller names and along the axis it names —
    /// for a fixture whose ink has to be a shape [`atlas`]'s opaque square is
    /// not, or whose travel is not the default one.
    fn draw_from(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        glyph: GlyphInstance,
        shadow: Option<harmonigraph_scene::ShadowStyle>,
        sheet: FontAtlas,
        slide: SlideAxis,
    ) -> Vec<u8> {
        draw_from_scaled(device, queue, glyph, shadow, sheet, slide, 1.0).0
    }

    fn draw_from_scaled(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        glyph: GlyphInstance,
        shadow: Option<harmonigraph_scene::ShadowStyle>,
        sheet: FontAtlas,
        slide: SlideAxis,
        ppp: f32,
    ) -> (Vec<u8>, [u32; 2]) {
        let physical_width = (SIZE[0] as f32 * ppp).round() as u32;
        let size = [physical_width.div_ceil(64) * 64, (SIZE[1] as f32 * ppp).round() as u32];
        let cb = TextCallback {
            glyphs: vec![glyph],
            shadow,
            atlas: Some(sheet),
            marks: None,
            sdf: Some(sdf_atlas()),
            slide,
            target_format: FORMAT,
            pane_id: 0,
            shadow_surface_id: shadow.map(|_| 0),
            pass_nr: 0,
        };
        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: size, pixels_per_point: ppp };
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(device, queue, &screen, &mut encoder, &mut resources);
        if cb.shadow_surface_id.is_some() {
            crate::spectral_shadow::finish(device, queue, &screen, &mut encoder, &mut resources, 0);
        }
        queue.submit(bufs.into_iter().chain([encoder.finish()]));

        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32));
        let texture =
            render_to_texture(device, queue, size, FORMAT, wgpu::Color::TRANSPARENT, |pass| {
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
            });
        (readback(device, queue, &texture, size), size)
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
            sdf_rect: [24.0, 24.0, 8.0, 8.0],
            sdf_near: [60.0, 60.0, 68.0, 68.0],
            sdf_coarse: [60.0, 60.0, 68.0, 68.0],
            fill: [255, 255, 255, 255],
            rim: [255, 0, 0, 255],
            atlas: GlyphInstance::TYPE,
        }
    }

    /// A true signed field for [`glyph`]'s opaque square, with enough room for
    /// both fixed sampling ranges around it.
    pub(crate) fn sdf_atlas() -> GlyphSdfAtlas {
        const SIDE: u32 = 128;
        let mut image = Vec::with_capacity((SIDE * SIDE) as usize);
        for y in 0..SIDE {
            for x in 0..SIDE {
                let p = glam::Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let q = (p - glam::Vec2::splat(64.0)).abs() - glam::Vec2::splat(4.0);
                let outside = q.max(glam::Vec2::ZERO).length();
                let inside = q.x.max(q.y).min(0.0);
                image.push(outside + inside);
            }
        }
        GlyphSdfAtlas { image: std::sync::Arc::new(image), size: [SIDE, SIDE], key: 1 }
    }

    /// The glyph lands where it was told to, in its own color, and the SDF
    /// shadow stands outside it in the pane's knockout color.
    ///
    /// The ink's own edge is soft by a quarter texel, which is `FILTER_TAP`'s
    /// outer tap and the price of a stroke that holds its weight as it
    /// slides. It is asserted here rather than tolerated, since the fringe is
    /// exactly as wide as that constant and a fringe any wider is the filter
    /// reaching somewhere it must not.
    #[test]
    fn a_glyph_paints_its_ink_and_the_sdf_shadow_stands_outside_it() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let shadow = harmonigraph_scene::ShadowStyle {
            width: 0.5,
            depth: 1.0,
            kernel: harmonigraph_scene::ShadowKernel::Distance,
        };
        let frame = draw(&device, &queue, glyph(), Some(shadow));
        assert_eq!(pixel(&frame, 28, 28), [255, 255, 255, 255], "the glyph itself");
        let outer = pixel(&frame, 22, 28);
        assert_eq!([outer[1], outer[2]], [0, 0], "the knockout's own hue, got {outer:?}");
        assert!(
            outer[3] > 0,
            "the Distance shadow did not reach two points past the glyph: {outer:?}",
        );
        assert_eq!(pixel(&frame, 4, 4), [0, 0, 0, 0], "nothing anywhere else");
    }

    /// Each instance reads the sheet it names, and only that one.
    ///
    /// The selector is the whole of what makes a drawn mark a glyph here: the
    /// two sheets are bound together and a fragment picks between them per
    /// instance. Both directions are drawn, because one alone passes for the
    /// wrong reason — a shader that ignored the flag and always read the font
    /// atlas would paint the letter correctly and lose the mark, and one that
    /// always read the marks would do the reverse.
    ///
    /// The two fixtures put their patches at coordinates the OTHER sheet
    /// leaves transparent, so reading the wrong one is a blank rather than a
    /// square of ink that happens to look right. The mark also sits against
    /// the sheet's own top wall, which is where the mark packer starts its
    /// first shelf — so `outside_atlas` is answering for this sheet's size,
    /// not for the font atlas's.
    #[test]
    fn a_glyph_reads_the_sheet_it_names() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let cb = TextCallback {
            // The letter where `glyph` puts it, the mark 16 points to its left.
            glyphs: vec![glyph(), GlyphInstance { rect: [8.0, 24.0, 8.0, 8.0], ..mark() }],
            shadow: None,
            atlas: Some(atlas()),
            marks: Some(mark_sheet()),
            sdf: None,
            slide: SlideAxis::default(),
            target_format: FORMAT,
            pane_id: 0,
            shadow_surface_id: None,
            pass_nr: 0,
        };
        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32));
        let texture =
            render_to_texture(&device, &queue, SIZE, FORMAT, wgpu::Color::TRANSPARENT, |pass| {
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
        let frame = readback(&device, &queue, &texture, SIZE);
        assert_eq!(pixel(&frame, 28, 28), [255, 255, 255, 255], "the letter, off the font atlas");
        assert_eq!(pixel(&frame, 12, 28), [255, 255, 255, 255], "the mark, off its own sheet");
    }

    /// A glyph packed against the atlas's own edge paints the same picture as
    /// one with epaint's transparent texel all the way around it.
    ///
    /// Which is not free, and is the reason `outside_atlas` exists. The margin
    /// `coverage` reads with looks half a texel outside the patch expecting to
    /// find that transparent texel; at the boundary there is none, and
    /// `ClampToEdge` answers with the glyph's own edge texel instead — the
    /// letter's top row drawn a second time above itself, with a seam under it.
    /// epaint fills its first band from row 0 and starts each band at column 0,
    /// so the edge case is where most glyphs live rather than where a few
    /// unlucky ones do.
    ///
    /// Three patches of one 8x8 square: against the top-left corner, padded in
    /// the middle, and against the bottom-right. All four boundaries, and the
    /// middle one is what the other two have to match.
    ///
    /// Drawn at a fractional offset because that is where a label actually
    /// lands, and it is the WEAKER of the two cases rather than the only one
    /// that fails. On the pixel grid a fragment centre falls on `texel = -0.5`
    /// exactly, so the tap's second sample carries a weight of zero and the
    /// clamp returns the edge texel whole: a full extra row of ink against this
    /// fixture's `[3, 0, 0, 3]` at a fractional offset. Both catch a missing
    /// fade; the fractional one is the picture the bug was reported from.
    ///
    /// The offsets differ by whole points and the patches sit at whole texels,
    /// so all three are at one sub-pixel phase and the pictures are comparable
    /// pixel for pixel. The atlas is deliberately NOT square, which is what
    /// holds the far wall to `atlas_size` per axis: read as a scalar it would
    /// put the bottom wall at the right one, and only a picture with two
    /// different walls can tell.
    #[test]
    fn a_glyph_against_the_atlas_edge_paints_what_a_padded_one_does() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // 32x48, so the far corner patch ends exactly on the last texel of both.
        let mut image = egui::ColorImage::filled([32, 48], egui::Color32::TRANSPARENT);
        for (left, top) in [(0, 0), (12, 12), (24, 40)] {
            for y in top..top + 8 {
                for x in left..left + 8 {
                    image[(x, y)] = egui::Color32::WHITE;
                }
            }
        }
        let cb = TextCallback {
            glyphs: [(4.3, 0.0, 0.0), (24.3, 12.0, 12.0), (44.3, 24.0, 40.0)]
                .iter()
                .map(|&(x, u, v)| GlyphInstance {
                    rect: [x, 24.3, 8.0, 8.0],
                    uv: [u, v, u + 8.0, v + 8.0],
                    ..glyph()
                })
                .collect(),
            shadow: None,
            atlas: Some(FontAtlas { image: std::sync::Arc::new(image), key: 1 }),
            marks: None,
            sdf: None,
            slide: SlideAxis::default(),
            target_format: FORMAT,
            pane_id: 0,
            shadow_surface_id: None,
            pass_nr: 0,
        };
        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32));
        let texture =
            render_to_texture(&device, &queue, SIZE, FORMAT, wgpu::Color::TRANSPARENT, |pass| {
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
        let frame = readback(&device, &queue, &texture, SIZE);

        // Everything the visible filter reaches, which is where the two edges
        // differ: the patch bound cuts a tap a whole texel out, so a doubled
        // row shows up in the fill.
        for dy in -3i32..12 {
            for dx in -3i32..12 {
                let at = |left: i32| pixel(&frame, (left + dx) as u32, (24 + dy) as u32);
                let (middle, corner, far) = (at(24), at(4), at(44));
                for (label, edge) in [("top-left", corner), ("bottom-right", far)] {
                    // A point of tolerance: the same arithmetic on three
                    // different atlas coordinates need not land on the same
                    // float. The doubled row it is here to catch is a quarter
                    // of the ink, not a rounding.
                    let off = (0..4).map(|c| edge[c].abs_diff(middle[c])).max().unwrap_or(0);
                    assert!(
                        off <= 1,
                        "the {label} glyph draws {edge:?} at ({dx}, {dy}) where the padded \
                         one draws {middle:?}",
                    );
                }
            }
        }
    }

    /// A pane whose text is already prepared keeps it when a LATER pane in the
    /// same frame brings a grown atlas.
    ///
    /// Which pane brings one is not a property of the pane: it is whichever
    /// happened to lay out a glyph nobody had drawn before, so on any frame the
    /// panes ahead of it in paint order have already had their `prepare` and
    /// will not get another before they are painted. Both halves of what
    /// [`TextResources::bind_sheets`] hands them are checked here, and each
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
        let at = |x: f32, pane_id: u64, atlas: Option<FontAtlas>| TextCallback {
            glyphs: vec![GlyphInstance { rect: [x, 24.0, 8.0, 8.0], ..glyph() }],
            shadow: None,
            atlas,
            marks: None,
            sdf: None,
            slide: SlideAxis::default(),
            target_format: FORMAT,
            pane_id,
            shadow_surface_id: None,
            pass_nr: 0,
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
            let texture = render_to_texture(
                &device,
                &queue,
                SIZE,
                FORMAT,
                wgpu::Color::TRANSPARENT,
                |pass| {
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
                },
            );
            readback(&device, &queue, &texture, SIZE)
        };

        // The first pane brings the atlas and the second rides on it, which is
        // the ordinary frame and the baseline the second one is read against.
        let first = frame(&mut resources, [at(8.0, 0, Some(atlas_of(32, 1))), at(40.0, 1, None)]);
        assert_eq!(pixel(&first, 12, 28), [255, 255, 255, 255], "the leading pane's glyph");
        assert_eq!(pixel(&first, 44, 28), [255, 255, 255, 255], "the trailing pane's glyph");

        // Now the other way round: the leading pane has nothing new to say and
        // the trailing one grows the atlas out from under it.
        let grown = frame(&mut resources, [at(8.0, 0, None), at(40.0, 1, Some(atlas_of(64, 2)))]);
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
        // takes `bind_sheets`'s early return, so nothing is handed to the
        // leading pane on this frame either.
        let reaching = GlyphInstance {
            rect: [8.0, 24.0, 8.0, 8.0],
            uv: [8.0, GROWN_PATCH_TOP as f32, 16.0, GROWN_PATCH_TOP as f32 + 8.0],
            ..glyph()
        };
        let deeper = frame(
            &mut resources,
            [
                TextCallback {
                    glyphs: vec![reaching],
                    shadow: None,
                    atlas: None,
                    marks: None,
                    sdf: None,
                    slide: SlideAxis::default(),
                    target_format: FORMAT,
                    pane_id: 0,
                    shadow_surface_id: None,
                    pass_nr: 1,
                },
                at(40.0, 1, Some(atlas_of(64, 3))),
            ],
        );
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

    /// Both explicit kernels derive a visible knockout from the same glyph
    /// SDF and leave the glyph fill on top of it.
    #[test]
    fn both_spectral_text_kernels_draw_from_the_glyph_sdf() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        for kernel in
            [harmonigraph_scene::ShadowKernel::Distance, harmonigraph_scene::ShadowKernel::Gaussian]
        {
            for ppp in [1.0f32, 1.5, 2.0, 4.0] {
                let style = harmonigraph_scene::ShadowStyle { width: 1.0, depth: 1.0, kernel };
                let (frame, size) = draw_from_scaled(
                    &device,
                    &queue,
                    glyph(),
                    Some(style),
                    atlas(),
                    SlideAxis::default(),
                    ppp,
                );
                let at = |x: f32, y: f32| {
                    let x = (x * ppp).floor() as u32;
                    let y = (y * ppp).floor() as u32;
                    let i = ((y * size[0] + x) * 4) as usize;
                    [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
                };
                assert_eq!(at(28.0, 28.0), [255, 255, 255, 255], "{kernel:?} fill at {ppp}");
                let outer = at(22.0, 28.0);
                assert!(
                    outer[3] > 0 && outer[0] > outer[1] && outer[0] > outer[2],
                    "{kernel:?} never drew the red shadow two points outside the glyph at {ppp} \
                     ppp: {outer:?}",
                );
                assert_eq!(
                    at(4.0, 4.0),
                    [0, 0, 0, 0],
                    "{kernel:?} escaped its atlas cell at {ppp} ppp",
                );
            }
        }
    }

    /// A spectral shadow is worth its color's alpha, which is the label's
    /// strength: half the strength, half the knockout.
    ///
    /// The reading the picture can be held to, since it is the fill's — a
    /// glyph at half strength covers half — and the two have to agree or a
    /// fading name is not one thing fading. Where they do not, the shadow is a
    /// dilation of the letter's own shape in the skin's darkest color, so
    /// what stands after the ink has gone is the name as a black silhouette.
    ///
    /// Measured against the shadow the panes draw rather than a model.
    #[test]
    fn a_faded_label_takes_its_shadow_with_it() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let shadow = harmonigraph_scene::ShadowStyle {
            width: 0.5,
            depth: 1.0,
            kernel: harmonigraph_scene::ShadowKernel::Distance,
        };
        // The shadow's own pass, with no fill over it: a pixel beside the
        // letter reads only the Distance profile.
        let shadow_at = |strength: f32| {
            let a = (strength * 255.0).round() as u8;
            let faded = GlyphInstance { fill: [0, 0, 0, 0], rim: [a, 0, 0, a], ..glyph() };
            pixel(&draw(&device, &queue, faded, Some(shadow)), 22, 28)[3]
        };
        // Every strength read before anything is asserted, so a failure
        // reports the whole curve: what the shape of the disagreement is says
        // where the level is being spent, and one reading does not carry it.
        let read: Vec<(f32, u8)> = [1.0f32, 0.5, 0.25, 0.1]
            .into_iter()
            .map(|strength| (strength, shadow_at(strength)))
            .collect();
        let full = read[0].1;
        assert!(
            read.iter().all(|(strength, shadow)| {
                shadow.abs_diff((f32::from(full) * strength).round() as u8) <= 2
            }),
            "a shadow profile should fade in its ink's proportion, got {read:?}",
        );
    }

    /// A label that slides moves smoothly: over a whole physical pixel of
    /// travel, no pixel of the picture moves further in one step than the
    /// step itself can account for.
    ///
    /// Every label on a picture pane walks this range. Positions arrive
    /// unrounded, deliberately — a name rides the thing it names instead of
    /// stepping across it while the thing glides (`harmonigraph_ui::text`) —
    /// so a name on a scrolling roll and a node name under a moving camera
    /// are both at a fresh sub-pixel offset every frame. What that costs has
    /// to be a resample and nothing more; anything that is a function of the
    /// offset with a STEP in it is a letter twitching as it travels, once per
    /// pixel it crosses.
    ///
    /// The margin in `coverage` is what this pins. A tap at the patch's own
    /// edge already reads half of its neighbouring texel, that being what a
    /// bilinear tap on a texel boundary is, so cutting there exactly puts a
    /// step of half the edge texel's coverage into the picture — the whole
    /// 128 levels of it on this fixture, whose glyph is opaque to its edge.
    ///
    /// The bound sits just beyond what sliding costs on its own. Real glyphs
    /// move by a small fraction of opaque through the same walk, which is what
    /// moving a sixteenth of a pixel means.
    #[test]
    fn a_glyph_slides_across_a_pixel_without_a_step() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        const STEPS: u32 = 16;
        let frames: Vec<Vec<u8>> = (0..STEPS)
            .map(|step| {
                let mut sliding = glyph();
                sliding.rect[0] += step as f32 / STEPS as f32;
                draw(&device, &queue, sliding, None)
            })
            .collect();
        let (worst, at) = (1..frames.len())
            .map(|i| {
                let step = frames[i - 1]
                    .chunks(4)
                    .zip(frames[i].chunks(4))
                    .map(|(a, b)| (0..4).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0))
                    .max()
                    .unwrap_or(0);
                (step, i)
            })
            .max()
            .expect("sixteen positions");
        assert!(
            worst <= 48,
            "a sixteenth of a pixel moved some pixel by {worst}/255, between position \
             {} and {at}: the picture steps where it should resample",
            at - 1,
        );
    }

    /// A sheet holding one HAIRLINE: a single opaque texel column, eight
    /// tall, inside the transparent texel epaint leaves around a glyph.
    ///
    /// [`atlas`]'s opaque square cannot stand in for it. A square's two
    /// vertical edges are eight texels apart, so each resamples on its own
    /// and the ink between them is never in doubt; a hairline's are the SAME
    /// texel's two sides, and it is that — one stroke about a pixel wide —
    /// that a small accidental is made of and that sub-pixel phase shows on.
    fn hairline_sheet() -> FontAtlas {
        let mut image = egui::ColorImage::filled([32, 32], egui::Color32::TRANSPARENT);
        for y in 8..16 {
            image[(8, y)] = egui::Color32::WHITE;
        }
        FontAtlas { image: std::sync::Arc::new(image), key: 2 }
    }

    /// The same hairline turned on its side: one opaque texel ROW, eight
    /// across, in the same transparent surround.
    ///
    /// What a stroke reads as when the motion is vertical, and `♯` is the
    /// mark it is drawn from — its two bars are horizontal, so they are to a
    /// roll running time DOWN the pane what the uprights are to one running it
    /// across. Sampling is separable, so nothing about the sideways reading
    /// carries over to this one: the two axes pass or fail independently, and
    /// that is the whole point of having both fixtures.
    fn hairline_sheet_across() -> FontAtlas {
        let mut image = egui::ColorImage::filled([32, 32], egui::Color32::TRANSPARENT);
        for x in 8..16 {
            image[(x, 8)] = egui::Color32::WHITE;
        }
        FontAtlas { image: std::sync::Arc::new(image), key: 3 }
    }

    /// A stroke a pixel wide keeps its weight as it slides, instead of
    /// tightening and loosening once per pixel it crosses.
    ///
    /// The complaint `FILTER_TAP` answers, and a different one from
    /// [`a_glyph_slides_across_a_pixel_without_a_step`]: that reads whether
    /// the picture JUMPS between two offsets, and a mark can walk perfectly
    /// smoothly while still being a different weight at each end of the walk.
    /// Smooth and steady are separate properties, and small type on a
    /// scrolling roll needs both.
    ///
    /// Read as `sum a(1-a)` over the frame — how much of the picture is
    /// partial coverage. A resample conserves the ink exactly, so this is not
    /// the mark getting heavier and lighter but its weight moving between
    /// being IN a pixel and being spread across two, which is the same
    /// quantity the mark bitmaps are tuned against
    /// (`harmonigraph_ui::marks`'s `Grid::breathing`).
    ///
    /// The fixture is the worst case the app can present: an opaque hairline,
    /// where one phase puts the whole stroke inside a single pixel and the
    /// next splits it in half. Through one tap that swings the full 100% —
    /// at the phase that lands the stroke on a pixel there is no partial
    /// coverage anywhere in the frame — and two taps a quarter texel apart
    /// hold it to 24.8%. The bound sits between, near the measurement: this
    /// is a claim about the FILTER, and a filter that quietly lost its second
    /// tap reads 100 rather than 40.
    ///
    /// No shadow, which is deliberate and is the whole of what this fixture
    /// isolates: the FILL read through the filter, with nothing else in the
    /// frame. The SDF shadow has its own kernel/scale coverage above, so this
    /// test stays about visible ink reconstruction alone.
    ///
    /// BOTH axes, because the filter is one-dimensional and separable, so a
    /// pass on one says nothing at all about the other. A stroke sliding along
    /// the axis the pane did NOT name reads the single tap's full swing — that
    /// is the whole of what [`SlideAxis`] carries. Running the sideways case
    /// alone leaves two of the analyzer's four orientations shimmering and
    /// reports a pass, which is the reading this fixture exists to deny.
    #[test]
    fn a_sliding_hairline_keeps_its_weight() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let ink = |rect: [f32; 4], uv: [f32; 4]| GlyphInstance {
            rect,
            uv,
            sdf_rect: rect,
            sdf_near: [0.0; 4],
            sdf_coarse: [0.0; 4],
            fill: [255, 255, 255, 255],
            rim: [0, 0, 0, 0],
            atlas: GlyphInstance::TYPE,
        };
        const STEPS: u32 = 16;
        // Each case: which way the pane says its text travels, the fixture at
        // rest, the sheet it is cut from, and which of `rect`'s two position
        // components the walk moves.
        let cases = [
            (
                SlideAxis::Across,
                ink([24.0, 24.0, 1.0, 8.0], [8.0, 8.0, 9.0, 16.0]),
                hairline_sheet(),
                0,
            ),
            (
                SlideAxis::Down,
                ink([24.0, 24.0, 8.0, 1.0], [8.0, 8.0, 16.0, 9.0]),
                hairline_sheet_across(),
                1,
            ),
        ];
        for (slide, hairline, sheet, axis) in cases {
            let smear: Vec<f32> = (0..STEPS)
                .map(|step| {
                    let mut sliding = hairline;
                    sliding.rect[axis] += step as f32 / STEPS as f32;
                    draw_from(
                        &device,
                        &queue,
                        sliding,
                        None,
                        FontAtlas { image: sheet.image.clone(), key: sheet.key },
                        slide,
                    )
                    .chunks(4)
                    .map(|px| {
                        let a = px[3] as f32 / 255.0;
                        a * (1.0 - a)
                    })
                    .sum()
                })
                .collect();
            let hi = smear.iter().copied().fold(0.0f32, f32::max);
            let lo = smear.iter().copied().fold(f32::INFINITY, f32::min);
            assert!(hi > 0.0, "the {slide:?} hairline drew nothing at any phase");
            let swing = (hi - lo) / hi;
            assert!(
                swing <= 0.40,
                "a hairline's partial coverage swings {:.1}% of itself across one pixel of \
                 travel ({lo:.2}..{hi:.2}) on {slide:?}: the stroke changes weight as it slides",
                100.0 * swing,
            );
        }
    }

    /// The lattice's four-tap mode reaches both axes, rather than quietly
    /// degenerating to either one of the two scrolling-pane filters.
    ///
    /// A fixture per axis is what makes this reach both pairs of taps. Each is
    /// deliberately moved along the axis an `Across`/`Down` filter pointed the
    /// other way would leave completely unfiltered; a one-axis implementation
    /// therefore reads the single tap's full swing on one of the two cases.
    #[test]
    fn the_two_axis_filter_reconstructs_motion_in_either_direction() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let ink = |rect: [f32; 4], uv: [f32; 4]| GlyphInstance {
            rect,
            uv,
            sdf_rect: rect,
            sdf_near: [0.0; 4],
            sdf_coarse: [0.0; 4],
            fill: [255, 255, 255, 255],
            rim: [0, 0, 0, 0],
            atlas: GlyphInstance::TYPE,
        };
        const STEPS: u32 = 16;
        let cases = [
            (ink([24.0, 24.0, 1.0, 8.0], [8.0, 8.0, 9.0, 16.0]), hairline_sheet(), 0),
            (ink([24.0, 24.0, 8.0, 1.0], [8.0, 8.0, 16.0, 9.0]), hairline_sheet_across(), 1),
        ];
        for (hairline, sheet, axis) in cases {
            let smear: Vec<f32> = (0..STEPS)
                .map(|step| {
                    let mut sliding = hairline;
                    sliding.rect[axis] += step as f32 / STEPS as f32;
                    draw_from(
                        &device,
                        &queue,
                        sliding,
                        None,
                        FontAtlas { image: sheet.image.clone(), key: sheet.key },
                        SlideAxis::Both,
                    )
                    .chunks(4)
                    .map(|px| {
                        let a = px[3] as f32 / 255.0;
                        a * (1.0 - a)
                    })
                    .sum()
                })
                .collect();
            let hi = smear.iter().copied().fold(0.0f32, f32::max);
            let lo = smear.iter().copied().fold(f32::INFINITY, f32::min);
            assert!(hi > 0.0, "the two-axis hairline on axis {axis} drew nothing at any phase");
            let swing = (hi - lo) / hi;
            assert!(
                swing <= 0.40,
                "the two-axis filter left axis {axis}'s hairline at {:.1}% swing ({lo:.2}..{hi:.2})",
                100.0 * swing,
            );
        }
    }
}
