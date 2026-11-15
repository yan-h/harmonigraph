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
/// Created and dropped as Background glow reach or gain crosses 0
/// (`lattice_frame`'s `lights`), independently of the resize
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

/// A frame's light table and the ink strips read from it: what every lit node
/// is putting on itself, read round each of them at [`INK_STRIP_N`] angles and
/// blurred there.
///
/// One row per lit instance, in this frame's order (see `LightRow`), and every
/// row written whole every frame: `fs_ink_strip` reads each row of the table
/// into `raw`, and `fs_ink_blur` convolves that into `blurred`, plus, in one
/// extra column, the same average at no concentration — the mean a node's
/// middle eases toward. Nothing in any of them is carried to the next frame, so
/// a strip rebuilt taller loses nothing and a row has no owner to keep.
///
/// Small: an f32 RGBA texel per angle per node ([`INK_STRIP_FORMAT`]), so a
/// lattice of 400 lit nodes spends about 0.8 MiB on the two textures (400 rows
/// of 129 texels at 16 bytes) and 54 KB on the table, beside the much larger
/// pane-sized half-float attachments. That is what the light costs in memory
/// to stop costing a whole reading of the node per lit fragment.
pub(super) struct InkStrip {
    /// The light table, [`rows`](Self::rows) `LightRow`s long.
    pub(super) lights: wgpu::Buffer,
    /// The table, as the reading pass takes it.
    pub(super) lights_bind_group: wgpu::BindGroup,
    pub(super) raw_view: wgpu::TextureView,
    /// The raw reading, as the blur takes it.
    pub(super) raw_bind_group: wgpu::BindGroup,
    pub(super) blurred_view: wgpu::TextureView,
    /// The blurred strip, as the light's own draw reads it.
    pub(super) blurred_bind_group: wgpu::BindGroup,
    /// How many rows the set was built for: the most nodes one frame has lit
    /// since the glow was last switched on, rounded up to a power of two
    /// ([`PaneBuffers::ensure_ink_strip`]).
    pub(super) rows: u32,
}

/// The two layouts an [`InkStrip`]'s bind groups are made against.
#[derive(Clone, Copy)]
pub(super) struct StripLayouts<'a> {
    /// A strip, as the blur and the light's own draw read it.
    pub(super) strip: &'a wgpu::BindGroupLayout,
    /// The light table, as the reading pass reads it.
    pub(super) light: &'a wgpu::BindGroupLayout,
}

/// The fewest rows a strip is built for, and the floor the doubling starts
/// from: 64 rows is 132 KB of strip and 9 KB of table, and enough that an
/// ordinary lattice never rebuilds them at all.
pub(super) const FIRST_ROWS: u32 = 64;

/// How many angles a node's ink is read at, and so how wide its strip is.
/// Mirrors `INK_STRIP_N` in lattice.wgsl, which is where the number is argued;
/// `the_shaders_ink_strip_is_as_wide_as_the_texture_it_is_drawn_into` is what
/// keeps the two one.
pub(super) const INK_STRIP_N: u32 = 64;

/// What the strip is kept in: an f32 colour per angle, which is what the blur
/// hands the light. The format is explicit rather than inherited from the
/// scene because a strip texel is a normalised colour beside a WEIGHT, and a
/// weight is a layer's level times its width — small numbers that must not be
/// quantized away if the scene format changes. Full precision since the strip
/// carried colour from frame to frame, where half precision stalled a held
/// halo short of its note's new colour (#1759, Q2); the strip is now written
/// whole every frame, and the format is kept because it costs a few hundred KB
/// and a weight read at half precision would round every halo by a little.
/// Every read is a `textureLoad`, so f32 needs no filterable-float feature.
pub(super) const INK_STRIP_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;

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
    /// alone. Neither transition touches the main target or the ink strip.
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
    /// function of its frame (`shadow::pack_into`), so the texture's size decides
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
    /// The set, for at least `rows` lit nodes: the next power of two, floored
    /// at [`FIRST_ROWS`]. Rows past the frame's lit nodes are neither written
    /// nor read — every pass over the strip stops at the frame's own count.
    pub(super) fn new(device: &wgpu::Device, layouts: StripLayouts<'_>, rows: u32) -> Self {
        #[cfg(test)]
        lattice_tests::INK_STRIP_CREATIONS.with(|count| count.set(count.get() + 1));
        let rows = rows.next_power_of_two().max(FIRST_ROWS);
        let lights = crate::gpu_memory::buffer(
            device,
            &wgpu::BufferDescriptor {
                label: Some("lattice_light_table"),
                size: u64::from(rows) * std::mem::size_of::<LightRow>() as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );
        let lights_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lattice_light_table_bind_group"),
            layout: layouts.light,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: lights.as_entire_binding() }],
        });
        let tex = |label: &str, width: u32| {
            crate::gpu_memory::texture(
                device,
                &wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width, height: rows, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: INK_STRIP_FORMAT,
                    // Read back by the tests, which check the reading itself.
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | if cfg!(test) {
                            wgpu::TextureUsages::COPY_SRC
                        } else {
                            wgpu::TextureUsages::empty()
                        },
                    view_formats: &[],
                },
            )
            .create_view(&Default::default())
        };
        // The blurred strip is one column wider than the reading it blurs: the
        // extra one holds the row's MEAN, which is the same convolution at no
        // concentration and so falls out of the same loop (`fs_ink_blur`).
        let raw_view = tex("lattice_ink_strip_raw", INK_STRIP_N);
        let blurred_view = tex("lattice_ink_strip", INK_STRIP_N + 1);
        let bind = |label: &str, view: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: layouts.strip,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                }],
            })
        };
        InkStrip {
            lights,
            lights_bind_group,
            raw_bind_group: bind("lattice_ink_strip_raw_bind_group", &raw_view),
            blurred_bind_group: bind("lattice_ink_strip_bind_group", &blurred_view),
            raw_view,
            blurred_view,
            rows,
        }
    }
}
