// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct Locals {
    metal::float4 shadow;
    metal::float4 cut;
    metal::float2 origin_points;
    metal::float2 viewport_points;
    float feather;
    float _feather_pad;
    metal::float2 pitch_dir;
    metal::float2 depth_dir;
    metal::float2 origin_pixels;
    metal::float2 pixel_points;
    float cut_fill;
    float _tail_pad;
};
struct CoverOut {
    metal::float4 position;
    metal::float2 uv;
    char _pad2[8];
};
struct GlowCut {
    metal::packed_float3 color;
    float weight;
    float covered;
    char _pad3[12];
};
constant uint OCCLUDER_HEADER = 5u;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;
constant float GLOW_BASE = 0.8;

float cut_ramp(
    float u,
    constant Locals& locals
) {
    float x = metal::clamp(u, 0.0, 1.0);
    float _e13 = locals.cut.z;
    return 1.0 - metal::mix(x, (x * x) * (3.0 - (2.0 * x)), _e13);
}

metal::float3 linear_from_gamma_rgb(
    metal::float3 srgb
) {
    metal::bool3 cutoff = srgb < metal::float3(0.04045);
    metal::float3 lower = srgb / metal::float3(12.92);
    metal::float3 higher = metal::pow((srgb + metal::float3(0.055)) / metal::float3(1.055), metal::float3(2.4));
    return metal::select(higher, lower, cutoff);
}

float cut_reference_at(
    metal::int2 texel,
    metal::depth2d<float, metal::access::sample> cut_reference
) {
    bool local = {};
    metal::int2 size = static_cast<metal::int2>(metal::uint2(cut_reference.get_width(), cut_reference.get_height()));
    if (!(metal::any(texel < metal::int2(0)))) {
        local = metal::any(texel >= size);
    } else {
        local = true;
    }
    bool _e14 = local;
    if (_e14) {
        return 1.0;
    }
    uint clamped_lod_e18 = metal::min(uint(0), cut_reference.get_num_mip_levels() - 1);
    float _e18 = cut_reference.read(metal::min(metal::uint2(texel), metal::uint2(cut_reference.get_width(clamped_lod_e18), cut_reference.get_height(clamped_lod_e18)) - 1), clamped_lod_e18);
    return _e18;
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

GlowCut glow_cut(
    CoverOut in_1,
    constant Locals& locals,
    metal::depth2d<float, metal::access::sample> cut_reference,
    metal::texture2d<float, metal::access::sample> glow_mask,
    metal::texture2d<float, metal::access::sample> cut_blend
) {
    float cut = 0.0;
    float u_1 = {};
    metal::int2 size_1 = static_cast<metal::int2>(metal::uint2(glow_mask.get_width(), glow_mask.get_height()));
    metal::int2 texel_1 = metal::min(naga_f2i32(in_1.uv * static_cast<metal::float2>(size_1)), as_type<metal::int2>(as_type<metal::uint2>(size_1) - as_type<metal::uint2>(metal::int2(1))));
    uint clamped_lod_e14 = metal::min(uint(0), glow_mask.get_num_mip_levels() - 1);
    metal::float4 glow = glow_mask.read(metal::min(metal::uint2(texel_1), metal::uint2(glow_mask.get_width(clamped_lod_e14), glow_mask.get_height(clamped_lod_e14)) - 1), clamped_lod_e14);
    float _e15 = cut_reference_at(texel_1, cut_reference);
    if (_e15 < 1.0) {
        u_1 = _e15;
        float kappa = locals.cut.w;
        if (kappa > 0.0) {
            uint clamped_lod_e29 = metal::min(uint(0), cut_blend.get_num_mip_levels() - 1);
            metal::float4 _e29 = cut_blend.read(metal::min(metal::uint2(texel_1), metal::uint2(cut_blend.get_width(clamped_lod_e29), cut_blend.get_height(clamped_lod_e29)) - 1), clamped_lod_e29);
            float sum = _e29.x;
            float tail = metal::exp(-((1.0 - _e15)) / kappa);
            u_1 = _e15 - (kappa * metal::log(metal::max(sum + tail, 0.000000000000000000000000000001)));
        }
        float _e45 = locals.cut.x;
        float _e46 = u_1;
        float _e47 = cut_ramp(_e46, locals);
        cut = _e45 * _e47;
    }
    float _e50 = cut;
    float covered = metal::max(glow.w, _e50);
    float _e56 = locals.cut_fill;
    return GlowCut {glow.xyz, metal::max(glow.w, _e56 * covered), covered};
}

struct fs_glow_cut_linearInput {
    metal::float2 uv [[user(loc0), center_perspective]];
};
struct fs_glow_cut_linearOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_glow_cut_linearOutput fs_glow_cut_linear(
  fs_glow_cut_linearInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Locals& locals [[buffer(0)]]
, metal::depth2d<float, metal::access::sample> cut_reference [[texture(1)]]
, metal::texture2d<float, metal::access::sample> glow_mask [[texture(0)]]
, metal::texture2d<float, metal::access::sample> cut_blend [[texture(2)]]
) {
    const CoverOut in = { position, varyings.uv };
    GlowCut _e1 = glow_cut(in, locals, cut_reference, glow_mask, cut_blend);
    metal::float3 _e3 = linear_from_gamma_rgb(_e1.color);
    return fs_glow_cut_linearOutput { metal::float4(_e3 * _e1.weight, _e1.covered) };
}
