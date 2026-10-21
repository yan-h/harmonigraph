//! The lattice's render pipelines: the scene's node, marker and label draws, the
//! shadow cells, the ink strips, the glow's splat and the post-process passes.

use super::*;

/// The three bind group layouts a scene pipeline draws through: the pane's
/// uniforms at group 0, the finished light at group 1 — the field a node washes
/// its own ink with (`node_paint`) — and the shadow atlas at group 2, the cell
/// each draw multiplies the frame by.
///
/// Both the node and the marker pipeline take all three: they are one pass over
/// one pane, so one layout is what lets the light and the atlas be bound once
/// for both. Whether there IS either to bind is the caller's business — see
/// `CompiledLatticeResources::glow_dummy_bind_group` and `shadow_dummy_bind_group`.
#[derive(Clone, Copy)]
pub(super) struct SceneLayouts<'a> {
    pub(super) uniforms: &'a wgpu::BindGroupLayout,
    pub(super) glow: &'a wgpu::BindGroupLayout,
    pub(super) shadow: &'a wgpu::BindGroupLayout,
    /// Every caster's kernel; see [`CompiledLatticeResources::caster_layout`].
    pub(super) casters: &'a wgpu::BindGroupLayout,
}

/// The lattice's own shader module, out of a WHOLE module's source: what
/// [`with_common`] builds at startup, or what the watcher reads back off disk
/// on a reload. Never lattice.wgsl alone, which names what common.wgsl
/// declares.
///
/// Every pipeline cut from that text shares one module per resource build, so
/// this is the one place the text becomes a module and the one place that
/// contract is stated.
pub(super) fn lattice_module(device: &wgpu::Device, shader_src: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("lattice_shader"),
        source: wgpu::ShaderSource::Wgsl(shader_src.into()),
    })
}

/// The visible pair, local notation transmission, then optional bloom input.
pub(super) fn scene_targets(
    format: wgpu::TextureFormat,
    bloom: bool,
) -> Vec<Option<wgpu::ColorTargetState>> {
    let target = |format| {
        Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })
    };
    let mut targets = vec![target(format), target(format), target(wgpu::TextureFormat::R16Float)];
    if bloom {
        targets.push(target(format));
    }
    targets
}

/// Build one of the scene pipelines from the module shared by this resource
/// build. Node and marker pipelines share the module, bind group layout,
/// blending, and topology; only entry points and vertex layout differ.
///
/// The scene writes two visible colors and scalar local transmission, plus
/// the bloom input when enabled (see [`LatticeBloom::input_view`]).
/// The single-attachment variant serves only direct rasterization references.
/// Both rely on painter order and carry no depth state.
fn create_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    target_format: wgpu::TextureFormat,
    layouts: SceneLayouts<'_>,
    entry_points: (&str, &str),
    vertex_layouts: &[wgpu::VertexBufferLayout<'_>],
    attachments: usize,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("lattice_pipeline_layout"),
        bind_group_layouts: &[
            Some(layouts.uniforms),
            Some(layouts.glow),
            Some(layouts.shadow),
            Some(layouts.casters),
        ],
        ..Default::default()
    });

    let color_target = wgpu::ColorTargetState {
        format: target_format,
        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        write_mask: wgpu::ColorWrites::ALL,
    };
    let targets = if attachments == 1 {
        vec![Some(color_target)]
    } else {
        scene_targets(target_format, attachments == 4)
    };

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        // Name the pipeline after its vertex entry point, so a GPU capture
        // can tell the node and marker passes apart.
        label: Some(entry_points.0),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(entry_points.0),
            compilation_options: Default::default(),
            buffers: vertex_layouts,
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(entry_points.1),
            compilation_options: Default::default(),
            targets: &targets,
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

/// One target for rasterization references, three for the visible scene plus
/// local transmission, four when the label-free bloom input is also needed.
pub(super) fn create_pipelines(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    target_format: wgpu::TextureFormat,
    layouts: SceneLayouts<'_>,
    attachments: usize,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let (node, plus) = match attachments {
        1 => ("fs_main", "fs_plus"),
        3 => ("fs_main_split", "fs_plus_split"),
        4 => ("fs_main_scene", "fs_plus_scene"),
        _ => unreachable!("scene attachment count"),
    };
    (
        create_pipeline(
            device,
            shader,
            target_format,
            layouts,
            ("vs_main", node),
            &[GpuInstance::LAYOUT, shadow::ShadowBox::BESIDE_NODES],
            attachments,
        ),
        create_pipeline(
            device,
            shader,
            target_format,
            layouts,
            ("vs_plus", plus),
            &[GpuPlus::LAYOUT],
            attachments,
        ),
    )
}

/// Attachment-compatible draws for a shared painter-order traversal.
/// The scene writes its visible pair, bloom input and local shadow transmission.
#[derive(Clone)]
pub(super) struct OrderedPipelines {
    pub(super) nodes: wgpu::RenderPipeline,
    pub(super) pluses: wgpu::RenderPipeline,
    /// Glyph ink or its coverage, depending on the pass.
    pub(super) glyph_fill: wgpu::RenderPipeline,
    /// Each label's shadow immediately precedes its ink in painter order.
    pub(super) shadow_box: wgpu::RenderPipeline,
}

#[derive(Clone)]
pub(super) struct ScenePipelines {
    pub(super) draws: OrderedPipelines,
    pub(super) glow_over: wgpu::RenderPipeline,
}

fn create_scene_pipelines(
    device: &wgpu::Device,
    lattice_shader: &wgpu::ShaderModule,
    blit_shader: &wgpu::ShaderModule,
    glyph_shader: &wgpu::ShaderModule,
    layouts: SceneLayouts<'_>,
    glyph_layout: &wgpu::BindGroupLayout,
) -> [ScenePipelines; 2] {
    [false, true].map(|bloom| {
        let (nodes, pluses) = create_pipelines(
            device,
            lattice_shader,
            LATTICE_COLOR_FORMAT,
            layouts,
            if bloom { 4 } else { 3 },
        );
        ScenePipelines {
            draws: OrderedPipelines {
                nodes,
                pluses,
                glyph_fill: text::create_text_pipeline(
                    device,
                    glyph_shader,
                    LATTICE_COLOR_FORMAT,
                    glyph_layout,
                    Some(layouts),
                    ("vs_glyph_lit", "fs_fill_lit"),
                    if bloom { 4 } else { 3 },
                    EGUI_BLEND,
                ),
                shadow_box: text::create_shadow_box_pipeline(
                    device,
                    glyph_shader,
                    glyph_layout,
                    layouts.glow,
                    layouts.shadow,
                    layouts.casters,
                    LATTICE_COLOR_FORMAT,
                    bloom,
                ),
            },
            glow_over: create_glow_over_pipeline(
                device,
                blit_shader,
                LATTICE_COLOR_FORMAT,
                layouts.glow,
                bloom,
            ),
        }
    })
}

/// The two draws that FILL the shadow atlas, from one source: a node's own ink
/// into that node's cell, and one cross into the markers' shared one.
///
/// Group 0 alone. Neither may bind the atlas — a texture cannot be read while
/// it is the target being written — so both take its size off `u.shadow_target`
/// instead, and neither reads the light: what a cell holds is coverage, and the
/// colour it is laid down in is settled where the cell is READ.
///
/// The node pipeline overwrites its cell so a negative analytic interior
/// survives the target's zero clear; one node owns one cell. The marker
/// pipeline is MAX-blended like the glyph pass beside it
/// (`text::create_glyph_cell_pipeline`) so repeated ink forms one union.
pub(super) fn create_cell_pipelines(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    uniforms: &wgpu::BindGroupLayout,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    const MAX_COMPONENT: wgpu::BlendComponent = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Max,
    };
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("lattice_cell_pipeline_layout"),
        bind_group_layouts: &[Some(uniforms)],
        ..Default::default()
    });
    let pipeline = |entries: (&str, &str),
                    buffers: &[wgpu::VertexBufferLayout<'_>],
                    blend: Option<wgpu::BlendState>| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(entries.0),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(entries.0),
                compilation_options: Default::default(),
                buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(entries.1),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: shadow::ATLAS_FORMAT,
                    blend,
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
    (
        pipeline(
            ("vs_node_cell", "fs_node_cell"),
            &[GpuInstance::LAYOUT, shadow::ShadowBox::BESIDE_NODES],
            // One node owns one cell. Overwrite lets an analytic distance keep
            // its negative interior; MAX against the clear value would clamp
            // every inside texel back to zero.
            None,
        ),
        // No instance data at all: the cell is one cross at the home sheet's
        // size, and what varies between markers is spent where it is read.
        pipeline(
            ("vs_plus_cell", "fs_plus_cell"),
            &[],
            Some(wgpu::BlendState { color: MAX_COMPONENT, alpha: MAX_COMPONENT }),
        ),
    )
}

/// The two passes that settle what colour every node's light is, ahead of the
/// draws that lay it down: the ink read round each node, then blurred.
///
/// **Why a pass at all.** `ink_at` is a function of the NODE and an angle —
/// no uv, no field, no derivative — so a fragment shader evaluating it was
/// answering the same question once per lit pixel, and a node's halo is a lot
/// of pixels. Here it is answered [`INK_STRIP_N`] times per node per frame
/// whatever the zoom, and the light's own draw reads two texels.
///
/// **Why two.** A blur cannot read the target it is writing. The first pass
/// lays the reading down over the instance buffer, one row per node; the second
/// convolves each row with the Spread bar's lobe.
///
/// **Both are over the INSTANCES**, each drawing the one row its node was
/// handed rather than a quad over the whole strip. A strip is as tall as the
/// row map's capacity and a frame lights whatever share of it it lights, so a
/// full-target quad would blur rows nothing wrote — and, worse for the reading
/// pass, would have no instance to take a row and a mix off.
///
/// **Timed scenes carry color**: a row averages current ink with the previous
/// strip (`InkStrip::carried`). The renderer computes each instance's mix from
/// the time since this pane last encoded an ink pass. This is deterministic
/// for offline rendering too. Fresh strips seed on their first frame; untimed
/// scenes always seed current ink as independent snapshots.
///
/// **No blending on either.** Each writes the row it draws, and a strip texel
/// is a colour and a weight rather than something to composite.
pub(super) fn create_ink_strip_pipelines(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    bind_group_layout: &wgpu::BindGroupLayout,
    strip_layout: &wgpu::BindGroupLayout,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let build = |label: &str,
                 entry_points: (&str, &str),
                 layout: &wgpu::PipelineLayout,
                 buffers: &[wgpu::VertexBufferLayout<'_>]| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(entry_points.0),
                compilation_options: Default::default(),
                buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(entry_points.1),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: INK_STRIP_FORMAT,
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
    // One layout for the two: each takes the node instances and one strip — the
    // reading takes the strip it is carrying from, the blur the strip that
    // reading has just left.
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("lattice_ink_strip_pipeline_layout"),
        bind_group_layouts: &[Some(bind_group_layout), Some(strip_layout)],
        ..Default::default()
    });
    (
        build("fs_ink_strip", ("vs_ink_strip", "fs_ink_strip"), &layout, &[GpuInstance::LAYOUT]),
        build("fs_ink_blur", ("vs_ink_blur", "fs_ink_blur"), &layout, &[GpuInstance::LAYOUT]),
    )
}

/// The draw that lays a finished glow target down at the bottom of the scene
/// pass, before any node, marker or label.
///
/// With bloom on it writes both the picture and the independent bloom input.
/// With bloom off it writes just the visible pair.
/// Both use painter order and the same premultiplied blend.
fn create_glow_over_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    target_format: wgpu::TextureFormat,
    light_layout: &wgpu::BindGroupLayout,
    bloom: bool,
) -> wgpu::RenderPipeline {
    // The light alone: this pass lays a finished field down and takes no
    // dial off the scene's uniforms.
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("lattice_glow_over_pipeline_layout"),
        bind_group_layouts: &[Some(light_layout)],
        ..Default::default()
    });
    let mut targets = scene_targets(target_format, bloom);
    targets[2].as_mut().unwrap().write_mask = wgpu::ColorWrites::empty();
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("fs_glow_over"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_blit"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(if bloom { "fs_glow_over" } else { "fs_glow_split" }),
            compilation_options: Default::default(),
            targets: &targets,
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

/// egui's own blend state, verbatim (see egui-wgpu's renderer): premultiplied
/// color, and alpha accumulated so the pass composites the same way over a
/// transparent framebuffer.
///
/// The three callbacks that draw their own geometry take it — the roll's
/// notes, the glyphs of [`crate::text`], and the halo of [`crate::glow`]. On a
/// halo that is what makes it pure LIGHT: it carries zero alpha, so the color
/// term adds and the alpha term leaves the destination's own alone.
///
/// One definition rather than one per callback, because them agreeing is what
/// makes them composite identically — the roll's notes over the spectrogram,
/// the spiral's halo over its disc — where copies agree only until one is
/// edited.
///
/// The lattice's `fs_composite` is the one thing here that does NOT name it,
/// and deliberately: it spells the same operator as
/// `PREMULTIPLIED_ALPHA_BLENDING`, which is `a(1-b)+b` where this is
/// `a+b(1-a)` — the same arithmetic written from the other side, as
/// [`crate::text::create_text_pipeline`]'s own doc sets out. Pointing it here
/// would rename a difference that is real in the source and absent in every
/// pixel.
pub(super) const EGUI_BLEND: wgpu::BlendState = wgpu::BlendState {
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
};

/// The fullscreen shader shared by one resource build's post-process and glow
/// pipelines. Not named for the lattice: the roll builds its own pipelines out
/// of the same source, so a validation error carrying the lattice's name would
/// send a reader to the wrong picture.
pub(super) fn blit_module(device: &wgpu::Device) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("blit_shader"),
        source: wgpu::ShaderSource::Wgsl(BLIT_SRC.into()),
    })
}

/// One post-process pipeline over the shared blit module: a fullscreen quad
/// with the given fragment entry point. The composite (into the egui pass)
/// blends premultiplied; the bloom-chain passes overwrite their whole target
/// and pass `blend: None`.
pub(super) fn create_post_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    entry_point: &str,
    target_format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    let bloom_layout = matches!(entry_point, "fs_bright" | "fs_blur_h" | "fs_blur_v")
        .then(|| BloomChain::scale_layout(device));
    let mut layouts = vec![Some(bind_group_layout)];
    if let Some(layout) = &bloom_layout {
        layouts.push(Some(layout));
    }
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("post_pipeline_layout"),
        bind_group_layouts: &layouts,
        ..Default::default()
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry_point),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_blit"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(entry_point),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend,
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

/// Every pipeline built from the reloadable lattice/text modules. Construction
/// and hot reload replace this value through the same complete pipeline list.
#[derive(Clone)]
pub(super) struct ShaderPipelines {
    pub(super) scenes: [ScenePipelines; 2],
    pub(super) glow_splat_pipeline: wgpu::RenderPipeline,
    pub(super) glow_resolve_pipeline: wgpu::RenderPipeline,
    pub(super) source_shadow_pipeline: wgpu::RenderPipeline,
    pub(super) ink_strip_pipeline: wgpu::RenderPipeline,
    pub(super) ink_blur_pipeline: wgpu::RenderPipeline,
    pub(super) glyph_cell_pipeline: wgpu::RenderPipeline,
    pub(super) node_cell_pipeline: wgpu::RenderPipeline,
    pub(super) plus_cell_pipeline: wgpu::RenderPipeline,
}

pub(super) struct ShaderLayouts<'a> {
    pub(super) scene: SceneLayouts<'a>,
    pub(super) glyph: &'a wgpu::BindGroupLayout,
    pub(super) strip: &'a wgpu::BindGroupLayout,
    pub(super) statistics: &'a wgpu::BindGroupLayout,
}

impl ShaderPipelines {
    pub(super) fn new(
        device: &wgpu::Device,
        lattice: &wgpu::ShaderModule,
        blit: &wgpu::ShaderModule,
        glyph: &wgpu::ShaderModule,
        layouts: ShaderLayouts<'_>,
        progress: impl Fn(startup::Stage),
    ) -> Self {
        let (node_cell_pipeline, plus_cell_pipeline) =
            create_cell_pipelines(device, lattice, layouts.scene.uniforms);
        progress(startup::Stage::Lighting);
        let (glow_splat_pipeline, glow_resolve_pipeline) = create_glow_pipelines(
            device,
            lattice,
            LATTICE_COLOR_FORMAT,
            layouts.scene.uniforms,
            layouts.strip,
            layouts.statistics,
        );
        let source_shadow_pipeline =
            lattice_node_glow::source_shadow_pipeline(device, lattice, layouts.scene.uniforms);
        let (ink_strip_pipeline, ink_blur_pipeline) =
            create_ink_strip_pipelines(device, lattice, layouts.scene.uniforms, layouts.strip);
        progress(startup::Stage::Lattice);
        let scenes =
            create_scene_pipelines(device, lattice, blit, glyph, layouts.scene, layouts.glyph);
        progress(startup::Stage::Labels);
        let glyph_cell_pipeline = text::create_glyph_cell_pipeline(device, glyph, layouts.glyph);
        Self {
            scenes,
            node_cell_pipeline,
            plus_cell_pipeline,
            glow_splat_pipeline,
            glow_resolve_pipeline,
            source_shadow_pipeline,
            ink_strip_pipeline,
            ink_blur_pipeline,
            glyph_cell_pipeline,
        }
    }
}
