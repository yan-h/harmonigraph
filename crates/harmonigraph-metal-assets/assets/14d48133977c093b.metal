// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct SdfOut {
    metal::float4 position;
    metal::float2 near_texel;
    metal::float2 coarse_texel;
    metal::float4 near_bounds;
    metal::float4 coarse_bounds;
    metal::float2 scales;
    char _pad6[8];
};
constant float DISTANCE_KIND = 1.0;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_FALLOFF_FLOOR = 0.35;
constant float SHADOW_TAIL = 4.0;
constant float SHADOW_FALLOFF_FREE = 0.64025325;
constant float SHADOW_STOP = 2.0;
constant float SHADOW_INVISIBLE_FOLDS = 1.5586027;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;
constant uint SHEET_MARK = 1u;
constant float PATCH_MARGIN = 0.5;
constant float FILTER_TAP = 0.25;
constant float SDF_NEAR_PAD = 32.0;
constant float SDF_COARSE_PAD = 48.0;
constant float SDF_NEAR_BLEND = 8.0;

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

float sdf_sample(
    metal::float2 texel,
    metal::float4 bounds,
    metal::texture2d<float, metal::access::sample> sdf_atlas
) {
    metal::float2 safe = metal::clamp(texel, bounds.xy + metal::float2(0.5), bounds.zw - metal::float2(0.5));
    metal::float2 p = safe - metal::float2(0.5);
    metal::int2 first = naga_f2i32(metal::ceil(bounds.xy));
    metal::int2 last = as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(metal::ceil(bounds.zw))) - as_type<metal::uint2>(metal::int2(1, 1)));
    metal::int2 lo = metal::clamp(naga_f2i32(metal::floor(p)), first, last);
    metal::int2 hi = metal::clamp(as_type<metal::int2>(as_type<metal::uint2>(lo) + as_type<metal::uint2>(metal::int2(1, 1))), first, last);
    metal::float2 f = metal::fract(p);
    uint clamped_lod_e35 = metal::min(uint(0), sdf_atlas.get_num_mip_levels() - 1);
    metal::float4 _e35 = sdf_atlas.read(metal::min(metal::uint2(lo), metal::uint2(sdf_atlas.get_width(clamped_lod_e35), sdf_atlas.get_height(clamped_lod_e35)) - 1), clamped_lod_e35);
    uint clamped_lod_e42 = metal::min(uint(0), sdf_atlas.get_num_mip_levels() - 1);
    metal::float4 _e42 = sdf_atlas.read(metal::min(metal::uint2(metal::int2(hi.x, lo.y)), metal::uint2(sdf_atlas.get_width(clamped_lod_e42), sdf_atlas.get_height(clamped_lod_e42)) - 1), clamped_lod_e42);
    float a = metal::mix(_e35.x, _e42.x, f.x);
    uint clamped_lod_e51 = metal::min(uint(0), sdf_atlas.get_num_mip_levels() - 1);
    metal::float4 _e51 = sdf_atlas.read(metal::min(metal::uint2(metal::int2(lo.x, hi.y)), metal::uint2(sdf_atlas.get_width(clamped_lod_e51), sdf_atlas.get_height(clamped_lod_e51)) - 1), clamped_lod_e51);
    uint clamped_lod_e55 = metal::min(uint(0), sdf_atlas.get_num_mip_levels() - 1);
    metal::float4 _e55 = sdf_atlas.read(metal::min(metal::uint2(hi), metal::uint2(sdf_atlas.get_width(clamped_lod_e55), sdf_atlas.get_height(clamped_lod_e55)) - 1), clamped_lod_e55);
    float b = metal::mix(_e51.x, _e55.x, f.x);
    return metal::mix(a, b, f.y);
}

float glyph_sdf_distance(
    SdfOut in_1,
    metal::texture2d<float, metal::access::sample> sdf_atlas
) {
    metal::float2 coarse_at = metal::clamp(in_1.coarse_texel, in_1.coarse_bounds.xy + metal::float2(0.5), in_1.coarse_bounds.zw - metal::float2(0.5));
    float _e14 = sdf_sample(coarse_at, in_1.coarse_bounds, sdf_atlas);
    float coarse = (_e14 * in_1.scales.y) + (metal::length(in_1.coarse_texel - coarse_at) * in_1.scales.y);
    float _e27 = sdf_sample(in_1.near_texel, in_1.near_bounds, sdf_atlas);
    float near = _e27 * in_1.scales.x;
    float near_edge = metal::min(metal::min((in_1.near_texel.x - in_1.near_bounds.x) - 0.5, (in_1.near_texel.y - in_1.near_bounds.y) - 0.5), metal::min((in_1.near_bounds.z - 0.5) - in_1.near_texel.x, (in_1.near_bounds.w - 0.5) - in_1.near_texel.y));
    return metal::mix(coarse, near, metal::smoothstep(0.0, SDF_NEAR_BLEND, near_edge));
}

struct fs_glyph_sdf_coverageInput {
    metal::float2 near_texel [[user(loc0), center_perspective]];
    metal::float2 coarse_texel [[user(loc1), center_perspective]];
    metal::float4 near_bounds [[user(loc2), flat]];
    metal::float4 coarse_bounds [[user(loc3), flat]];
    metal::float2 scales [[user(loc4), flat]];
};
struct fs_glyph_sdf_coverageOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_glyph_sdf_coverageOutput fs_glyph_sdf_coverage(
  fs_glyph_sdf_coverageInput varyings [[stage_in]]
, metal::float4 position [[position]]
, metal::texture2d<float, metal::access::sample> sdf_atlas [[texture(2)]]
) {
    const SdfOut in = { position, varyings.near_texel, varyings.coarse_texel, varyings.near_bounds, varyings.coarse_bounds, varyings.scales };
    float _e1 = glyph_sdf_distance(in, sdf_atlas);
    float _e2 = metal::fwidth(_e1);
    float aa = metal::max(_e2, 0.000001);
    return fs_glyph_sdf_coverageOutput { metal::float4(metal::clamp(0.5 - (_e1 / aa), 0.0, 1.0), 0.0, 0.0, 1.0) };
}
