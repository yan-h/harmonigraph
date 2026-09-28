// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint size2;
};

struct ShadowCaster {
    metal::float4 rect;
    metal::float4 cell;
    metal::float4 map;
    metal::float4 shade;
};
typedef ShadowCaster type_6[1];
struct CompositeParams {
    float darkest_pitch;
    float brightest_pitch;
    float render_scale;
    float bloom_strength;
    metal::float4 background;
};
struct CameraParams {
    metal::float4x4 view_proj;
    metal::float4 right;
    metal::float4 up;
};
struct NodeParams {
    float radius;
    float band_inner;
    float band_outer;
    float rings_outer;
    float mark_inner;
    float angular_gap;
    float mark_thickness;
    float animation;
    metal::float4 pose;
};
struct MarkerParams {
    float half_width;
    float taper_start;
    float world_unit;
    float padding;
};
struct type_10 {
    metal::float4 inner[3];
};
struct OctaveParams {
    float span;
    float center;
    metal::float2 padding;
    type_10 bounds;
};
struct SpectralParams {
    float inner;
    float outer;
    float range_cents;
    float folded;
};
struct GlowParams {
    float reach;
    float strength;
    float padding;
    float curve;
    float wash;
    float row_capacity;
    float lit;
    float accumulation;
};
struct TextureParams {
    float depth;
    float scale;
    metal::float2 drift;
    metal::float2 target_size;
    uint style;
    float padding;
};
struct MaterialParams {
    float amount;
    float scale;
    metal::float2 drift;
    uint style;
    float fuzz;
    float lobe;
    float variety;
    float refract;
    float layers;
    float shadow_pickup;
    float padding;
};
struct ShadowParams {
    float width;
    float reach_sigmas;
    float depth;
    float occlusion;
};
struct ShadowTargetParams {
    metal::float2 pane_points;
    metal::float2 atlas_texels;
};
struct MarkerCellParams {
    metal::float4 rect;
    metal::float4 cell;
    float points_to_texels;
    float aa_scale;
    float arm_points;
    float spread_points;
};
struct type_11 {
    metal::float4 inner[64];
};
struct type_13 {
    metal::uint4 inner[240];
};
struct type_14 {
    metal::float4 inner[16];
};
struct Uniforms {
    CompositeParams composite;
    CameraParams camera;
    NodeParams node;
    MarkerParams marker;
    OctaveParams octave;
    SpectralParams spectral;
    GlowParams glow;
    TextureParams texture;
    MaterialParams material;
    ShadowParams geometry_shadow;
    ShadowParams marker_shadow;
    ShadowTargetParams shadow_target;
    MarkerCellParams marker_cell;
    metal::float4 lattice_ground;
    metal::float4 lut_spacing;
    type_11 pitch_lut;
    type_11 spectral_lut;
    type_13 spectrum_color;
    type_14 ink_kernel;
};
struct ShadowThrough {
    float seen;
    float bloom;
};
struct VsOut {
    metal::float4 clip_pos;
    metal::float2 uv;
    char _pad2[8];
    metal::float4 params;
    metal::uint3 octaves;
    metal::uint3 thickness;
    metal::uint4 motion;
    float cents;
    float strip_row;
    metal::uint2 marks;
    metal::float4 melody_color;
    metal::float4 bass_color;
    float rim;
    float swell;
    float ring;
    float ink_carry;
    metal::float4 shadow_box;
    metal::float4 shadow_at;
};
constant float DISTANCE_KIND = 1.0;
constant float DISTANCE_COVERAGE_KIND = 2.0;
constant uint OCCLUDER_HEADER = 5u;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_STOP = 1.0;
constant float SHADOW_FALLOFF_MIN = -6.0;
constant float SHADOW_FALLOFF_MAX = 6.0;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;
constant float TAU = 6.2831855;
constant float QUAD_MARGIN = 1.6;
constant float GLYPH_FADE_LIMIT = 1.3;
constant float GLOW_BASE = 0.8;
constant float SHADOW_REACH_SIGMAS = 3.0;
constant uint INK_STRIP_N = 64u;
constant bool EARLY_OUT = true;
constant float INK_FLOOR = 0.01;
constant uint OCTAVE_SLOTS = 11u;
constant float THICKNESS_STEPS = 64.0;
constant float GAUSSIAN_SPREAD_KIND = 3.0;
constant uint MAX_SPAN = 11u;
constant uint PITCH_LUT_N = 64u;
constant float BEND_ROUNDING = 0.4;
constant uint SPECTRUM_BUCKETS = 3828u;
constant float BUCKETS_PER_SEMITONE = 32.0;
constant float SPECTRUM_MIN_MIDI = 15.48682;
constant float AA_SOFTNESS_PX = 2.0;
constant float OCT_UP = -4.712389;
constant float DISTANCE_LEVEL_FLOOR = 0.5;
constant float EMPTY_DISTANCE = 65504.0;
constant float PLUS_QUAD_MARGIN = 1.6;
constant metal::float3 GLOW_LUMINANCE = metal::float3(0.2126, 0.7152, 0.0722);

bool cell_packed(
    metal::float4 cell
) {
    bool local = {};
    if (cell.z > 0.0) {
        local = cell.w > 0.0;
    } else {
        local = false;
    }
    bool _e10 = local;
    return _e10;
}

float standoff_coverage(
    float d,
    float w,
    float falloff
) {
    float u_1 = metal::clamp((metal::max(d, 0.0) / metal::max(w, 0.000001)) / SHADOW_STOP, 0.0, 1.0);
    float remaining = 1.0 - u_1;
    float shape = -(metal::clamp(falloff, SHADOW_FALLOFF_MIN, SHADOW_FALLOFF_MAX));
    if (metal::abs(shape) < 0.05) {
        return remaining * ((1.0 - ((shape * u_1) * 0.5)) + ((((shape * shape) * u_1) * ((2.0 * u_1) - 1.0)) / 12.0));
    }
    return (metal::exp(shape * remaining) - 1.0) / (metal::exp(shape) - 1.0);
}

float shadow_kernel(
    uint who,
    metal::float2 points,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_6 const& shadow_casters,
    constant _mslBufferSizes& _buffer_sizes
) {
    if (who >= (1 + (_buffer_sizes.size2 - 0 - 64) / 64)) {
        return 0.0;
    }
    metal::float4 cell_1 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size2 - 0 - 64) / 64)].cell;
    bool _e10 = cell_packed(cell_1);
    if (!(_e10)) {
        return 0.0;
    }
    metal::float2 atlas = static_cast<metal::float2>(metal::uint2(shadow_atlas.get_width(), shadow_atlas.get_height()));
    metal::float4 map = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size2 - 0 - 64) / 64)].map;
    metal::float2 texel = metal::clamp(map.xy + (points * map.z), cell_1.xy + metal::float2(0.5), (cell_1.xy + cell_1.zw) - metal::float2(0.5));
    metal::float4 _e39 = shadow_atlas.sample(shadow_sampler, texel / atlas, metal::level(0.0));
    float held = _e39.x;
    float _e45 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size2 - 0 - 64) / 64)].shade.y;
    if (_e45 == DISTANCE_COVERAGE_KIND) {
        return metal::clamp(held, 0.0, 1.0);
    }
    float _e55 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size2 - 0 - 64) / 64)].shade.y;
    if (_e55 >= 0.5) {
        float _e62 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size2 - 0 - 64) / 64)].shade.z;
        float _e69 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size2 - 0 - 64) / 64)].shade.w;
        float _e70 = standoff_coverage(held, 2.0 * _e62, _e69);
        return metal::clamp(_e70, 0.0, 1.0);
    }
    return metal::min(GAUSSIAN_GAIN * metal::clamp(held, 0.0, 1.0), 1.0);
}

float glow_shadow_depth(
    constant Uniforms& u
) {
    float _e3 = u.geometry_shadow.depth;
    return metal::clamp(_e3, 0.0, 1.0);
}

uint naga_f2u32(float value) {
    return static_cast<uint>(metal::clamp(value, 0.0, 4294967000.0));
}

ShadowThrough node_shadow_through(
    float who_1,
    metal::float2 points_1,
    float level,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_6 const& shadow_casters,
    constant Uniforms& u,
    constant _mslBufferSizes& _buffer_sizes
) {
    if (level <= 0.0) {
        return ShadowThrough {1.0, 1.0};
    }
    float _e14 = shadow_kernel(naga_f2u32(metal::max(who_1, 0.0)), points_1, shadow_atlas, shadow_sampler, shadow_casters, _buffer_sizes);
    float coverage = metal::clamp(level, 0.0, 1.0) * _e14;
    float _e16 = glow_shadow_depth(u);
    return ShadowThrough {1.0 - (_e16 * coverage), 1.0 - coverage};
}

struct fs_source_shadowInput {
    metal::float2 uv [[user(loc0), center_perspective]];
    metal::float4 params [[user(loc2), center_perspective]];
    metal::uint3 octaves [[user(loc3), flat]];
    metal::uint3 thickness [[user(loc1), flat]];
    metal::uint4 motion [[user(loc9), flat]];
    float cents [[user(loc4), flat]];
    float strip_row [[user(loc5), flat]];
    metal::uint2 marks [[user(loc6), flat]];
    metal::float4 melody_color [[user(loc7), flat]];
    metal::float4 bass_color [[user(loc8), flat]];
    float rim [[user(loc11), flat]];
    float swell [[user(loc13), flat]];
    float ring [[user(loc14), flat]];
    float ink_carry [[user(loc15), flat]];
    metal::float4 shadow_box [[user(loc10), flat]];
    metal::float4 shadow_at [[user(loc12), center_no_perspective]];
};
struct fs_source_shadowOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_source_shadowOutput fs_source_shadow(
  fs_source_shadowInput varyings [[stage_in]]
, metal::float4 clip_pos [[position]]
, metal::texture2d<float, metal::access::sample> shadow_atlas [[texture(1)]]
, metal::sampler shadow_sampler [[sampler(1)]]
, device type_6 const& shadow_casters [[buffer(1)]]
, constant Uniforms& u [[buffer(0)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    const VsOut in = { clip_pos, varyings.uv, {}, varyings.params, varyings.octaves, varyings.thickness, varyings.motion, varyings.cents, varyings.strip_row, varyings.marks, varyings.melody_color, varyings.bass_color, varyings.rim, varyings.swell, varyings.ring, varyings.ink_carry, varyings.shadow_box, varyings.shadow_at };
    ShadowThrough _e7 = node_shadow_through(in.shadow_box.x, in.shadow_at.xy, in.shadow_at.z, shadow_atlas, shadow_sampler, shadow_casters, u, _buffer_sizes);
    float _e14 = u.material.shadow_pickup;
    return fs_source_shadowOutput { metal::float4(0.0, 0.0, 0.0, (1.0 - _e7.seen) * _e14) };
}
