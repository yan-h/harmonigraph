//! The lattice's per-pane offscreen targets: the scene and bloom attachments,
//! the node glow's target and the ink strips it reads.

use super::*;

/// The per-pane offscreen render target and bloom chain, recreated when
/// the pane's pixel size (or render scale) changes.
///
/// The scene target uses the render-scaled size; the bloom textures use
/// fractions of the pane's NATIVE screen size, so the halo's on-screen
/// width doesn't change with the render-scale setting.
pub(super) struct Offscreen {
    /// The descriptor format of every scene colour attachment but the R16F
    /// local transmission.
    #[cfg(test)]
    pub(super) format: wgpu::TextureFormat,
    pub(super) color_view: wgpu::TextureView,
    /// Local notation transmission, written beside scene ink in painter order.
    /// Bloom samples this finished field without blurring it.
    pub(super) local_shadow_view: wgpu::TextureView,
    /// Node and label RGB, spared ordinary node shadows and summed at composite.
    pub(super) ink_view: wgpu::TextureView,
    /// The independent label-free scene and its filtered halo.
    /// Present only while bloom is on; toggling it never replaces glow history.
    pub(super) bloom: Option<LatticeBloom>,
    /// The node glow's own target, present only while the view asks for one.
    pub(super) glow: Option<GlowTarget>,
    /// The names' shadow atlas, present only while a frame has names casting a
    /// shadow (`ensure_shadow`).
    ///
    /// Its own lifetime rather than a member of [`GlowTarget`], because the two
    /// answer to different bars: a name's shadow lands on the ground at a Reach
    /// of 0, where there is no light and no glow target at all.
    pub(super) shadow: Option<shadow::ShadowTarget>,
    /// Composite: background + node ink + blurred bloom (quarter A) + uniforms.
    pub(super) composite_bind_group: wgpu::BindGroup,
    pub(super) size: [u32; 2],
    pub(super) screen_size: [u32; 2],
}

/// The allocations whose contents are needed only while bloom is enabled.
pub(super) struct LatticeBloom {
    /// The label-free picture the bright pass thresholds: background light and
    /// node ink in one target. The visible picture splits these two so that
    /// ordinary shadows darken one and not the other; no shadow reaches this
    /// one, so it holds only their premultiplied sum, which is all the bright
    /// pass ever read of a pair.
    pub(super) input_view: wgpu::TextureView,
    pub(super) chain: BloomChain,
}

/// The glow target's share of the scene's width and height, as a divisor.
///
/// The glow is a smooth, low-frequency field, and everything sharp beside it
/// (rims, wedges, names, star cores) is drawn at full resolution on top, so a
/// quarter-size target reconstructs it within what is visible. Against half
/// size it saved about 1.3 ms at 6 notes and 2.5 ms at 24 notes at 2048²,
/// with no difference to see in the DAW (#1458).
pub(super) const GLOW_DIVISOR: u32 = 4;

/// Where a frame's node light is assembled before any of it reaches the
/// picture: one transparent premultiplied colour texture at a quarter of the
/// scene's width and height ([`GLOW_DIVISOR`]), plus the bind group its
/// readers take it through.
/// Stars keeps this source resolution but supplies a scene-resolution material
/// output through the same reader interface, preserving its compact cores.
///
/// A target of its own, rather than the glow drawn straight into the scene
/// pass, because a node has to sample the finished light to paint its own
/// picture (`node_paint`), and a pass cannot sample the attachment it writes.
/// Every node's halo melds here first (`fs_glow_resolve`), across every sheet at once,
/// and the scene pass then lays that one layer down at its bottom and reads it
/// again per node.
///
/// Created and dropped as the Reach bar crosses 0, independently of the resize
/// that rebuilds everything around it: the two changes have nothing to do with
/// each other, and a target left allocated at reach 0 is a glow-sized texture
/// held for a feature that is off.
pub(super) struct GlowTarget {
    /// The texels the scene's size over [`GLOW_DIVISOR`] came to. Every
    /// texture below, and the material source, is this size.
    pub(super) size: [u32; 2],
    pub(super) material_source: Option<lattice_material::Source>,
    pub(super) statistics: [wgpu::TextureView; 3],
    pub(super) statistics_bind_group: wgpu::BindGroup,
    /// The descriptor format of `view`.
    #[cfg(test)]
    pub(super) format: wgpu::TextureFormat,
    pub(super) view: wgpu::TextureView,
    /// The texture + the shared sampler, as
    /// [`CompiledLatticeResources::filter_layout`] takes them.
    bind_group: wgpu::BindGroup,
}

/// A frame's ink strips: what every node is putting on itself, read round each
/// of them at [`INK_STRIP_N`] angles and blurred there.
///
/// One row per assigned glow identity, indexed by the node's `strip_row`;
/// instance order may change without changing that identity. Two raw textures
/// carry history and a third holds its convolution: `raw` is
/// `fs_ink_strip`'s reading, `blurred` is
/// `fs_ink_blur`'s convolution of it plus, in one extra column, the same
/// average at no concentration — the mean a node's middle eases toward.
///
/// Small: an f16 RGBA texel per angle per node, so a lattice of 400 lit nodes
/// spends about 600 KiB on the three textures, beside the much larger pane-sized
/// half-float attachments. That is what the light costs in memory to stop
/// costing a whole reading of the node per lit fragment.
pub(super) struct InkStrip {
    pub(super) history: ink_history::InkHistory,
    /// The raw reading, in a PAIR that ping-pongs: the frame writes one and
    /// reads the other, which is what lets a row hold an average of this
    /// frame's ink and the ink that same row already had (`fs_ink_strip`).
    ///
    /// A node's light is carried on a clock of its own, and the COLOUR half of
    /// that is here — the ink is read in WGSL by the same functions that draw
    /// each layer, so there is nowhere else it could be carried without
    /// spelling every layer's colour a second time in Rust. Which is also why
    /// the row a node writes has to be the row it wrote last frame: this is a
    /// texture read back by identity, and the identity is the row
    /// (`harmonigraph_scene::GlowStep::row`).
    pub(super) raw_views: [wgpu::TextureView; 2],
    /// Each of the two, as a texture to read: `[parity]` is what the blur takes
    /// and `[parity ^ 1]` is last frame's, which the reading pass mixes into.
    pub(super) raw_bind_groups: [wgpu::BindGroup; 2],
    pub(super) blurred_view: wgpu::TextureView,
    /// The blurred strip, as the light's own draw reads it. Not a pair: the
    /// blur is a pure function of the raw strip that has just been written, so
    /// there is nothing in it to carry.
    pub(super) blurred_bind_group: wgpu::BindGroup,
    /// How many rows the set was built for — the row map's capacity on the
    /// frame that built it, which is what [`PaneBuffers::ensure_ink_history`] compares.
    pub(super) rows: u32,
    /// Which of [`raw_views`](Self::raw_views) this frame writes. Flipped once
    /// per frame, in `prepare`.
    pub(super) parity: usize,
}

/// How many angles a node's ink is read at, and so how wide its strip is.
/// Mirrors `INK_STRIP_N` in lattice.wgsl, which is where the number is argued;
/// `the_shaders_ink_strip_is_as_wide_as_the_texture_it_is_drawn_into` is what
/// keeps the two one.
pub(super) const INK_STRIP_N: u32 = 64;

/// What the strip is kept in: an f16 colour per angle, which is what the blur
/// hands the light. The format is explicit rather than inherited from the
/// scene because a strip texel is a normalised colour beside a WEIGHT, and a
/// weight is a layer's level times its width — small numbers that must not be
/// quantized away if the scene format changes.
pub(super) const INK_STRIP_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The shared, pane-independent objects an [`Offscreen`] binds against.
pub(super) struct OffscreenShared<'a> {
    pub(super) format: wgpu::TextureFormat,
    pub(super) composite_layout: &'a wgpu::BindGroupLayout,
    /// One texture plus the sampler; see [`CompiledLatticeResources::filter_layout`].
    pub(super) filter_layout: &'a wgpu::BindGroupLayout,
    /// The shadow atlas as its readers take it; see
    /// [`CompiledLatticeResources::shadow_layout`].
    pub(super) shadow_layout: &'a wgpu::BindGroupLayout,
    pub(super) sampler: &'a wgpu::Sampler,
    /// Transparent stand-in for the composite's bloom binding while off.
    pub(super) bloom_dummy: &'a wgpu::TextureView,
    pub(super) glow_statistics_layout: &'a wgpu::BindGroupLayout,
}

impl Offscreen {
    pub(super) fn new(
        device: &wgpu::Device,
        shared: &OffscreenShared<'_>,
        uniform_buffer: &wgpu::Buffer,
        size: [u32; 2],
        screen_size: [u32; 2],
    ) -> Self {
        let OffscreenShared { format, .. } = *shared;
        let tex = |label, w: u32, h: u32, format, usage| {
            crate::gpu_memory::texture(
                device,
                &wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                },
            )
        };
        let attach_and_sample =
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;

        let color = tex("lattice_offscreen_color", size[0], size[1], format, attach_and_sample);
        let color_view = color.create_view(&Default::default());
        let ink_view = tex("lattice_offscreen_ink", size[0], size[1], format, attach_and_sample)
            .create_view(&Default::default());
        let local_shadow_view = tex(
            "lattice_local_shadow",
            size[0],
            size[1],
            wgpu::TextureFormat::R16Float,
            attach_and_sample,
        )
        .create_view(&Default::default());
        let composite_bind_group = Self::composite_binding(
            device,
            shared,
            uniform_buffer,
            &color_view,
            &ink_view,
            &local_shadow_view,
            shared.bloom_dummy,
        );

        Offscreen {
            #[cfg(test)]
            format,
            bloom: None,
            glow: None,
            shadow: None,
            composite_bind_group,
            color_view,
            ink_view,
            local_shadow_view,
            size,
            screen_size,
        }
    }

    fn composite_binding(
        device: &wgpu::Device,
        shared: &OffscreenShared<'_>,
        uniforms: &wgpu::Buffer,
        color: &wgpu::TextureView,
        ink: &wgpu::TextureView,
        local_shadow: &wgpu::TextureView,
        bloom: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lattice_composite_bind_group"),
            layout: shared.composite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(color),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(shared.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(bloom),
                },
                wgpu::BindGroupEntry { binding: 3, resource: uniforms.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(ink),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(local_shadow),
                },
            ],
        })
    }

    /// Keyed only on enabled/disabled: sizes are fixed by this Offscreen's
    /// lifetime. A strength change within the enabled range updates uniforms
    /// alone. Neither transition touches the main target or the ink history.
    pub(super) fn ensure_bloom(
        &mut self,
        device: &wgpu::Device,
        shared: &OffscreenShared<'_>,
        uniforms: &wgpu::Buffer,
        want: bool,
    ) {
        if want == self.bloom.is_some() {
            return;
        }
        self.bloom = want.then(|| {
            let input_view = crate::gpu_memory::texture(
                device,
                &wgpu::TextureDescriptor {
                    label: Some("lattice_bloom_input"),
                    size: wgpu::Extent3d {
                        width: self.size[0],
                        height: self.size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: shared.format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            )
            .create_view(&Default::default());
            let chain = BloomChain::new(
                device,
                "lattice",
                shared.format,
                shared.filter_layout,
                shared.sampler,
                &input_view,
                self.screen_size,
            );
            LatticeBloom { input_view, chain }
        });
        self.composite_bind_group = Self::composite_binding(
            device,
            shared,
            uniforms,
            &self.color_view,
            &self.ink_view,
            &self.local_shadow_view,
            self.bloom.as_ref().map_or(shared.bloom_dummy, |b| &b.chain.quarter_a_view),
        );
    }

    /// Make this pane's quarter-resolution light target exist while `want`
    /// says so. The separate pane history is maintained by the caller under
    /// the same guard; recreating this image never allocates or transfers a
    /// strip.
    pub(super) fn ensure_glow(
        &mut self,
        device: &wgpu::Device,
        shared: &OffscreenShared<'_>,
        want: bool,
    ) {
        match (want, self.glow.is_some()) {
            (true, false) => self.glow = Some(GlowTarget::new(device, shared, self.size)),
            (false, true) => self.glow = None,
            _ => {}
        }
    }

    /// Make this pane's shadow atlas hold `want` texels — none at all where
    /// `want` is `None` — the bargain [`ensure_glow`](Offscreen::ensure_glow)
    /// strikes, on the names' shadows instead of the Reach.
    ///
    /// Grown to demand and never shrunk while it is wanted: a layout is a pure
    /// function of its frame (`shadow::pack`), so the texture's size decides
    /// nothing drawn, and a smaller one rebuilt on every frame a name leaves
    /// would be an allocation per frame of a fade. Nothing is carried across a
    /// rebuild the way a glow's ink strip is: the atlas is rewritten from the
    /// ink up every frame it is used.
    pub(super) fn ensure_shadow(
        &mut self,
        device: &wgpu::Device,
        shared: &OffscreenShared<'_>,
        want: Option<[u32; 2]>,
    ) {
        match want {
            Some(size) => shadow::ShadowTarget::ensure(
                &mut self.shadow,
                device,
                shared.shadow_layout,
                shared.sampler,
                size,
                "lattice",
            ),
            None => self.shadow = None,
        }
    }
}

impl GlowTarget {
    /// Stars preserve native scene-resolution cores; all other materials read the quarter-resolution light.
    pub(super) fn binding(&self) -> &wgpu::BindGroup {
        self.material_source
            .as_ref()
            .and_then(|source| source.star_output())
            .unwrap_or(&self.bind_group)
    }
    /// The scene's width and height over [`GLOW_DIVISOR`], rounded up for
    /// panes that do not divide evenly. All readers reconstruct the same
    /// filtered field in normalized coordinates.
    pub(super) fn new(
        device: &wgpu::Device,
        shared: &OffscreenShared<'_>,
        scene: [u32; 2],
    ) -> Self {
        let size = scene.map(|n| n.div_ceil(GLOW_DIVISOR).max(1));
        let OffscreenShared { format, filter_layout, sampler, .. } = *shared;
        let view = crate::gpu_memory::texture(
            device,
            &wgpu::TextureDescriptor {
                label: Some("lattice_glow"),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
        )
        .create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lattice_glow_bind_group"),
            layout: filter_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        let (statistics, statistics_bind_group) =
            lattice_node_glow::statistics(device, shared.glow_statistics_layout, size);
        GlowTarget {
            size,
            material_source: None,
            statistics,
            statistics_bind_group,
            #[cfg(test)]
            format,
            view,
            bind_group,
        }
    }
}

impl InkStrip {
    /// The set, for a strip `rows` tall. `rows` is floored at one: a texture of
    /// zero height is not a texture, and a spare row nothing draws into is
    /// simply never sampled — the light's own draw is over the instances, each
    /// of which reads the row it just wrote.
    ///
    /// A strip built here holds NOTHING, which is why the frame that builds one
    /// seeds rather than mixing: its new history has no initialized rows,
    /// whatever the last CPU layout pass supplied as its coefficient.
    pub(super) fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, rows: u32) -> Self {
        #[cfg(test)]
        lattice_tests::INK_STRIP_CREATIONS.with(|count| count.set(count.get() + 1));
        let tex = |label: &str, width: u32| {
            crate::gpu_memory::texture(
                device,
                &wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width, height: rows.max(1), depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: INK_STRIP_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            )
            .create_view(&Default::default())
        };
        // One column wider than the reading it blurs: the extra one holds the
        // row's MEAN, which is the same convolution at no concentration and so
        // falls out of the same loop (`fs_ink_blur`).
        let raw_views = [
            tex("lattice_ink_strip_raw_a", INK_STRIP_N),
            tex("lattice_ink_strip_raw_b", INK_STRIP_N),
        ];
        let blurred_view = tex("lattice_ink_strip", INK_STRIP_N + 1);
        let bind = |label: &str, view: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                }],
            })
        };
        InkStrip {
            raw_bind_groups: [
                bind("lattice_ink_strip_raw_a_bind_group", &raw_views[0]),
                bind("lattice_ink_strip_raw_b_bind_group", &raw_views[1]),
            ],
            blurred_bind_group: bind("lattice_ink_strip_bind_group", &blurred_view),
            raw_views,
            blurred_view,
            rows,
            parity: 0,
            history: ink_history::InkHistory::new(rows),
        }
    }

    /// The raw strip this frame writes...
    pub(super) fn writing(&self) -> &wgpu::TextureView {
        &self.raw_views[self.parity]
    }

    /// ...the one it wrote LAST frame, which is what a row's ink is carried
    /// from...
    pub(super) fn carried(&self) -> &wgpu::BindGroup {
        &self.raw_bind_groups[self.parity ^ 1]
    }

    /// ...and the one it has just written, which the blur reads.
    pub(super) fn written(&self) -> &wgpu::BindGroup {
        &self.raw_bind_groups[self.parity]
    }
}
