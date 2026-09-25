// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct CompositeParams {
    float darkest_pitch;
    float brightest_pitch;
    float render_scale;
    float bloom_strength;
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
struct type_8 {
    metal::float4 inner[3];
};
struct OctaveParams {
    float span;
    float center;
    metal::float2 padding;
    type_8 bounds;
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
    float blend;
    float curve;
    float wash;
    float row_capacity;
    float lit;
    float accumulation;
};
struct NebulaParams {
    float depth;
    float scale;
    metal::float2 drift;
    metal::float2 target_size;
    metal::float2 padding;
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
    float padding;
};
struct type_9 {
    metal::float4 inner[64];
};
struct type_11 {
    metal::uint4 inner[240];
};
struct type_12 {
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
    NebulaParams nebula;
    ShadowParams geometry_shadow;
    ShadowParams marker_shadow;
    ShadowTargetParams shadow_target;
    MarkerCellParams marker_cell;
    metal::float4 lattice_ground;
    metal::float4 lut_spacing;
    type_9 pitch_lut;
    type_9 spectral_lut;
    type_11 spectrum_color;
    type_12 ink_kernel;
};
struct GlowSplatOut {
    metal::float4 position;
    metal::float2 uv;
    metal::float2 light;
};
struct GlowStatistics {
    metal::float4 sum;
    metal::float2 screen;
    char _pad2[8];
    metal::float4 accumulated;
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

float node_rim(
    bool marked,
    constant Uniforms& u
) {
    float rim = {};
    bool local = {};
    float _e4 = u.node.rings_outer;
    rim = metal::max(_e4, 0.0);
    if (marked) {
        float _e13 = u.node.mark_thickness;
        local = _e13 > 0.0;
    } else {
        local = false;
    }
    bool _e17 = local;
    if (_e17) {
        float _e18 = rim;
        float _e22 = u.node.mark_inner;
        float _e26 = u.node.mark_thickness;
        rim = metal::max(_e18, _e22 + _e26);
    }
    float _e29 = rim;
    return _e29;
}

float glow_rim(
    constant Uniforms& u
) {
    float _e1 = node_rim(true, u);
    return _e1;
}

float glow_level(
    float carried
) {
    return metal::clamp(carried, 0.0, 1.0);
}

int naga_mod(int lhs, int rhs) {
    int divisor = metal::select(rhs, 1, (lhs == (-2147483647 - 1) & rhs == -1) | (rhs == 0));
    return lhs - (lhs / divisor) * divisor;
}

metal::float4 strip_texel(
    int col,
    int row,
    metal::texture2d<float, metal::access::sample> ink_strip
) {
    uint clamped_lod_e9 = metal::min(uint(0), ink_strip.get_num_mip_levels() - 1);
    metal::float4 _e9 = ink_strip.read(metal::min(metal::uint2(metal::int2(naga_mod(as_type<int>(as_type<uint>(naga_mod(col, 64)) + as_type<uint>(64)), 64), row)), metal::uint2(ink_strip.get_width(clamped_lod_e9), ink_strip.get_height(clamped_lod_e9)) - 1), clamped_lod_e9);
    return _e9;
}

int naga_f2i32(float value) {
    return static_cast<int>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

metal::float4 glow_ink(
    float strip_row,
    float angle,
    float mix_out,
    metal::texture2d<float, metal::access::sample> ink_strip
) {
    int row_1 = naga_f2i32(strip_row);
    float x = ((angle / TAU) * 64.0) - 0.5;
    float base = metal::floor(x);
    metal::float4 _e12 = strip_texel(naga_f2i32(base), row_1, ink_strip);
    metal::float4 _e16 = strip_texel(as_type<int>(as_type<uint>(naga_f2i32(base)) + as_type<uint>(1)), row_1, ink_strip);
    metal::float4 lit = metal::mix(_e12, _e16, x - base);
    uint clamped_lod_e23 = metal::min(uint(0), ink_strip.get_num_mip_levels() - 1);
    metal::float4 mean = ink_strip.read(metal::min(metal::uint2(metal::int2(64, row_1)), metal::uint2(ink_strip.get_width(clamped_lod_e23), ink_strip.get_height(clamped_lod_e23)) - 1), clamped_lod_e23);
    return metal::float4(metal::mix(mean.xyz, lit.xyz, mix_out), mean.w);
}

float glow_curve_at(
    float d,
    float span,
    constant Uniforms& u
) {
    float p = metal::clamp(d / span, 0.0, 1.0);
    float shape = u.glow.curve;
    float remaining = 1.0 - p;
    if (metal::abs(shape) < 0.05) {
        float shape2_ = shape * shape;
        return remaining * ((1.0 - ((shape * p) * 0.5)) + (((shape2_ * p) * ((2.0 * p) - 1.0)) / 12.0));
    }
    return (metal::exp(shape * remaining) - 1.0) / (metal::exp(shape) - 1.0);
}

metal::float4 glow_layer(
    metal::float2 light,
    metal::float2 uv,
    constant Uniforms& u,
    metal::texture2d<float, metal::access::sample> ink_strip
) {
    float _e3 = glow_level(light.x);
    float _e7 = u.glow.reach;
    float reach = metal::max(_e7, 0.0);
    float _e13 = u.glow.strength;
    float strength = metal::max(_e13, 0.0);
    float _e16 = glow_rim(u);
    float d_1 = metal::length(uv);
    float span_1 = metal::max(_e16 + reach, 0.1);
    if (d_1 >= span_1) {
        return metal::float4(0.0);
    }
    float _e25 = glow_curve_at(d_1, span_1, u);
    float skirt = GLOW_BASE * _e25;
    float _e28 = node_rim(false, u);
    float seam = metal::max(_e28, 0.1);
    float mix_out_1 = metal::min(1.0, (d_1 * d_1) / (seam * seam));
    float alpha = metal::clamp((skirt * _e3) * strength, 0.0, 1.0);
    if (alpha <= 0.0) {
        return metal::float4(0.0);
    }
    metal::float4 _e49 = glow_ink(light.y, metal::atan2(uv.y, uv.x), mix_out_1, ink_strip);
    if (_e49.w <= 0.0) {
        return metal::float4(0.0);
    }
    return metal::float4(_e49.xyz, alpha);
}

metal::float3 glow_linear(
    metal::float3 rgb
) {
    return metal::select(metal::pow((rgb + metal::float3(0.055)) / metal::float3(1.055), metal::float3(2.4)), rgb / metal::float3(12.92), rgb <= metal::float3(0.04045));
}

struct fs_glow_splatInput {
    metal::float2 uv [[user(loc0), center_perspective]];
    metal::float2 light [[user(loc1), flat]];
};
struct fs_glow_splatOutput {
    metal::float4 sum [[color(0)]];
    metal::float2 screen [[color(1)]];
    metal::float4 accumulated [[color(2)]];
};
fragment fs_glow_splatOutput fs_glow_splat(
  fs_glow_splatInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Uniforms& u [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> ink_strip [[texture(0)]]
) {
    const GlowSplatOut in = { position, varyings.uv, varyings.light };
    metal::float4 _e3 = glow_layer(in.light, in.uv, u, ink_strip);
    if (_e3.w <= 0.0) {
        metal::discard_fragment();
    }
    metal::float4 incoming = metal::float4(_e3.xyz * _e3.w, _e3.w);
    float _e16 = u.glow.strength;
    float peak = metal::clamp(GLOW_BASE * _e16, 0.0, 1.0);
    metal::float3 _e22 = glow_linear(metal::float3(peak));
    float peak_luminance = _e22.x;
    metal::float3 _e25 = glow_linear(incoming.xyz);
    float share = metal::clamp(metal::dot(_e25, GLOW_LUMINANCE) / peak_luminance, 0.0, 1.0);
    const auto _tmp = GlowStatistics {metal::float4(_e25, 1.0), metal::float2(share, _e3.w / peak), {}, incoming};
    return fs_glow_splatOutput { _tmp.sum, _tmp.screen, _tmp.accumulated };
}
