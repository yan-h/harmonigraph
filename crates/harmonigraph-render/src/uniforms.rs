//! Lattice GPU transport. Named scalars carry settings; vectors carry axes,
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
        impl layout::GpuLayout for $name {
            fn layout() -> layout::Layout {
                layout::Layout::of::<Self>(layout::Kind::Struct(vec![
                    $(layout::Field {
                        name: stringify!($field),
                        offset: std::mem::offset_of!(Self, $field),
                        layout: <$ty as layout::GpuLayout>::layout(),
                    }),*
                ]))
            }
        }
    };
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Float4(pub(super) [f32; 4]);

#[repr(C, align(16))]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Uint4(pub(super) [u32; 4]);

#[repr(C, align(8))]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Float2(pub(super) [f32; 2]);

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
        padding: f32,
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
        /// Clockwise angles from the ring seam, four boundaries per row.
        bounds: [Float4; 3],
    }
}
uniform_group! {
    struct ShimmerParams {
        /// Travel in world units, already reduced onto one cycle for f32 precision.
        slide: f32,
        period: f32,
        intensity: f32,
        softness: f32,
        pattern: f32,
        padding0: f32,
        padding1: f32,
        padding2: f32,
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
        blend: f32,
        curve: f32,
        wash: f32,
        /// Allocated row capacity, independent of this frame's instance count.
        row_capacity: f32,
        padding: Float2,
    }
}
uniform_group! {
    /// Shadows still cast without glow. Markers inherit the text group's style.
    struct ShadowParams {
        width: f32,
        reach_sigmas: f32,
        depth: f32,
        padding: f32,
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
        padding: f32,
    }
}
uniform_group! {
    struct Uniforms {
        composite: CompositeParams,
        camera: CameraParams,
        node: NodeParams,
        marker: MarkerParams,
        octave: OctaveParams,
        shimmer: ShimmerParams,
        spectral: SpectralParams,
        glow: GlowParams,
        geometry_shadow: ShadowParams,
        marker_shadow: ShadowParams,
        shadow_target: ShadowTargetParams,
        marker_cell: MarkerCellParams,
        lattice_ground: Float4,
        pitch_lut: [Float4; harmonigraph_scene::PITCH_LUT_N],
        spectral_lut: [Float4; harmonigraph_scene::PITCH_LUT_N],
        /// Analyzer levels through the colour dB window, sixteen bytes per row.
        /// Per-pane uniforms avoid an extra texture/upload and cross-view aliasing.
        spectrum_color: [Uint4; super::SPECTRUM_WORDS],
    }
}

#[cfg(test)]
pub(super) mod layout;
