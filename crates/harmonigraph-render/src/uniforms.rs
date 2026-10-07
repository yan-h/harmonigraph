//! GPU uniform transport. Named scalars carry settings; vectors carry axes,
//! colours and coordinates. The declaration also supplies test metadata from
//! the actual Rust field types and offsets, rather than a parallel schema.

macro_rules! uniform_group {
    ($(#[$doc:meta])* struct $name:ident { $($(#[$field_doc:meta])* $field:ident: $ty:ty),* $(,)? }) => {
        $(#[$doc])*
        #[repr(C, align(16))]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        pub(super) struct $name {
            $($(#[$field_doc])* pub(super) $field: $ty),*
        }

        #[cfg(test)]
        impl $crate::uniforms::layout::GpuLayout for $name {
            fn layout() -> $crate::uniforms::layout::Layout {
                $crate::uniforms::layout::Layout::of::<Self>($crate::uniforms::layout::Kind::Struct(vec![
                    $($crate::uniforms::layout::Field {
                        name: stringify!($field),
                        offset: std::mem::offset_of!(Self, $field),
                        layout: <$ty as $crate::uniforms::layout::GpuLayout>::layout(),
                    }),*
                ]))
            }
        }
    };
}

pub(crate) use uniform_group;

#[repr(C, align(16))]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Float4(pub(super) [f32; 4]);

#[repr(C, align(16))]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Uint4(pub(super) [u32; 4]);

#[repr(C, align(8))]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Float2(pub(super) [f32; 2]);

#[repr(C, align(8))]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Int2(pub(super) [i32; 2]);

/// Column-major, matching WGSL's matrix columns and glam's upload order.
#[repr(C, align(16))]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Matrix4(pub(super) [Float4; 4]);

uniform_group! {
    /// Binding 3 in blit.wgsl reads exactly this first group of Uniforms.
    struct CompositeParams {
        darkest_pitch: f32,
        brightest_pitch: f32,
        render_scale: f32,
        bloom_strength: f32,
        /// Pane fill participates in the local display-range cap.
        background: Float4,
        edge_softness_pixels: f32,
        padding: f32,
        padding2: Float2,
    }
}
uniform_group! {
    struct CameraParams {
        view_proj: Matrix4,
        right: Float4,
        up: Float4,
    }
}
uniform_group! {
    /// Ring and mark radii are in node quad UV; radius alone is in world units.
    /// Radial padding is already spent in these radii by the scene builder.
    struct NodeParams {
        radius: f32,
        band_inner: f32,
        band_outer: f32,
        rings_outer: f32,
        mark_inner: f32,
        angular_gap: f32,
        mark_thickness: f32,
        animation: f32,
        pose: Float4,
    }
}
uniform_group! {
    struct MarkerParams {
        half_width: f32,
        taper_start: f32,
        /// One node UV in world units. Needed for marker shadows even with glow off.
        world_unit: f32,
        padding: f32,
    }
}
uniform_group! {
    struct OctaveParams {
        span: f32,
        center: f32,
        padding: Float2,
        /// Slice `i`'s turn back from its ring's seam, see [`OctaveParams::turns`].
        turns: [Float4; harmonigraph_scene::MAX_SPAN as usize + 1],
    }
}
impl OctaveParams {
    /// Every slice's turn off its ring's seam as (cos, sin), for `span` slices
    /// to the turn: the first edge's, TAU * i / span, in xy and the middle's,
    /// half a slice on, in zw. The angles are the same for every ring, so the
    /// shader rotates each ring's seam by these rather than taking a sine per
    /// slot of every fragment (`oct_sector`). Held to the span the shader
    /// clamps to, so the two tables cannot disagree.
    pub(super) fn turns(span: u32) -> [Float4; harmonigraph_scene::MAX_SPAN as usize + 1] {
        let span = f64::from(span.clamp(1, harmonigraph_scene::MAX_SPAN));
        std::array::from_fn(|i| {
            let edge = std::f64::consts::TAU * i as f64 / span;
            let mid = std::f64::consts::TAU * (i as f64 + 0.5) / span;
            Float4([edge.cos(), edge.sin(), mid.cos(), mid.sin()].map(|v| v as f32))
        })
    }
}
uniform_group! {
    struct SpectralParams {
        inner: f32,
        outer: f32,
        range_cents: f32,
        folded: f32,
    }
}
uniform_group! {
    /// Zeroed when reach or strength disables glow. Reach is the shared draw predicate.
    struct GlowParams {
        reach: f32,
        strength: f32,
        padding: f32,
        curve: f32,
        wash: f32,
        /// Allocated row capacity, independent of this frame's instance count.
        row_capacity: f32,
        /// Whether this frame has any lit halo instances.
        lit: f32,
        accumulation: f32,
    }
}
uniform_group! {
    /// A shared texture modulates the completed glow, never the ink history.
    struct TextureParams {
        depth: f32,
        scale: f32,
        drift: Float2,
        target_size: Float2,
        /// The glow target's own texels, which `vs_source_shadow` pads its
        /// quad by; 1x1 where there is no target, which nothing then reads.
        glow_size: Float2,
    }
}
uniform_group! {
    struct PickupParams {
        intensity: f32,
        width: f32,
        softness: f32,
        color: f32,
    }
}
uniform_group! {
    /// Shadows still cast without glow. Markers inherit the text group's style.
    struct ShadowParams {
        width: f32,
        reach_sigmas: f32,
        depth: f32,
        /// How much a caster of this group hides the ink of the lattice nodes
        /// and names behind it, 0..=1: the Hide behind bar
        /// (`ViewConfig::hide_behind`) for the geometry group, and 0 for the
        /// markers, which hide nothing.
        occlusion: f32,
    }
}
uniform_group! {
    struct ShadowTargetParams {
        pane_points: Float2,
        /// Known only after packing; a draw cannot sample the atlas it is filling.
        atlas_texels: Float2,
    }
}
uniform_group! {
    /// One Gaussian cell serves all resting markers. Distance shadows leave it empty.
    struct MarkerCellParams {
        rect: Float4,
        cell: Float4,
        points_to_texels: f32,
        aa_scale: f32,
        arm_points: f32,
        spread_points: f32,
    }
}
uniform_group! {
    struct Uniforms {
        composite: CompositeParams,
        camera: CameraParams,
        node: NodeParams,
        marker: MarkerParams,
        octave: OctaveParams,
        spectral: SpectralParams,
        glow: GlowParams,
        texture: TextureParams,
        pickup: PickupParams,
        geometry_shadow: ShadowParams,
        marker_shadow: ShadowParams,
        shadow_target: ShadowTargetParams,
        marker_cell: MarkerCellParams,
        lattice_ground: Float4,
        /// `pitch_lut`'s spacing corner in xy, `spectral_lut`'s in zw.
        lut_spacing: Float4,
        pitch_lut: [Float4; harmonigraph_scene::PITCH_LUT_N],
        spectral_lut: [Float4; harmonigraph_scene::PITCH_LUT_N],
        /// Analyzer levels through the colour dB window, sixteen bytes per row.
        /// Per-pane uniforms avoid an extra texture/upload and cross-view aliasing.
        spectrum_color: [Uint4; super::SPECTRUM_WORDS],
        /// Circular blur weights by absolute angular sample offset.
        ink_kernel: [Float4; super::INK_STRIP_N as usize / 4],
    }
}

#[cfg(test)]
pub(super) mod layout;
