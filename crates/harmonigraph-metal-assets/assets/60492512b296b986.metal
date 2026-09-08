// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct Locals {
    metal::float2 screen_points;
    metal::float2 atlas_size;
    metal::float2 mark_atlas_size;
    metal::float2 filter_axis;
    float pixels_per_point;
    float shadow_depth;
    metal::float2 shadow_atlas_size;
    metal::float4 _pad;
};
struct VertexOut {
    metal::float4 position;
    metal::float2 texel;
    metal::float2 uv_min;
    metal::float2 uv_max;
    char _pad4[8];
    metal::float4 fill;
    uint sheet;
    char _pad6[4];
    metal::float2 sheet_size;
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

float outside_atlas(
    VertexOut in_1,
    metal::float2 texel
) {
    metal::float2 low = metal::clamp(texel + metal::float2(0.5), metal::float2(0.0), metal::float2(1.0));
    metal::float2 high = metal::clamp((in_1.sheet_size + metal::float2(0.5)) - texel, metal::float2(0.0), metal::float2(1.0));
    metal::float2 weight = low * high;
    return weight.x * weight.y;
}

float sheet_alpha(
    VertexOut in_2,
    metal::float2 texel_1,
    metal::texture2d<float, metal::access::sample> atlas,
    metal::sampler atlas_sampler,
    metal::texture2d<float, metal::access::sample> mark_atlas
) {
    metal::float2 uv = texel_1 / in_2.sheet_size;
    if (in_2.sheet == SHEET_MARK) {
        metal::float4 _e10 = mark_atlas.sample(atlas_sampler, uv, metal::level(0.0));
        return _e10.w;
    }
    metal::float4 _e15 = atlas.sample(atlas_sampler, uv, metal::level(0.0));
    return _e15.w;
}

float tap(
    VertexOut in_3,
    metal::float2 texel_2,
    metal::texture2d<float, metal::access::sample> atlas,
    metal::sampler atlas_sampler,
    metal::texture2d<float, metal::access::sample> mark_atlas
) {
    bool local = {};
    bool local_1 = {};
    bool local_2 = {};
    if (!((texel_2.x < (in_3.uv_min.x - PATCH_MARGIN)))) {
        local = texel_2.y < (in_3.uv_min.y - PATCH_MARGIN);
    } else {
        local = true;
    }
    bool _e18 = local;
    if (!(_e18)) {
        local_1 = texel_2.x > (in_3.uv_max.x + PATCH_MARGIN);
    } else {
        local_1 = true;
    }
    bool _e29 = local_1;
    if (!(_e29)) {
        local_2 = texel_2.y > (in_3.uv_max.y + PATCH_MARGIN);
    } else {
        local_2 = true;
    }
    bool _e40 = local_2;
    if (_e40) {
        return 0.0;
    }
    float _e42 = sheet_alpha(in_3, texel_2, atlas, atlas_sampler, mark_atlas);
    float _e43 = outside_atlas(in_3, texel_2);
    return _e42 * _e43;
}

float coverage(
    VertexOut in_4,
    metal::float2 texel_3,
    constant Locals& locals,
    metal::texture2d<float, metal::access::sample> atlas,
    metal::sampler atlas_sampler,
    metal::texture2d<float, metal::access::sample> mark_atlas
) {
    metal::float2 _e4 = locals.filter_axis;
    if (metal::all(_e4 > metal::float2(0.0))) {
        metal::float2 x = metal::float2(0.25, 0.0);
        metal::float2 y = metal::float2(0.0, 0.25);
        float _e17 = tap(in_4, (texel_3 - x) - y, atlas, atlas_sampler, mark_atlas);
        float _e20 = tap(in_4, (texel_3 + x) - y, atlas, atlas_sampler, mark_atlas);
        float _e24 = tap(in_4, (texel_3 - x) + y, atlas, atlas_sampler, mark_atlas);
        float _e28 = tap(in_4, (texel_3 + x) + y, atlas, atlas_sampler, mark_atlas);
        return 0.25 * (((_e17 + _e20) + _e24) + _e28);
    }
    metal::float2 _e35 = locals.filter_axis;
    metal::float2 off = FILTER_TAP * _e35;
    float _e38 = tap(in_4, texel_3 - off, atlas, atlas_sampler, mark_atlas);
    float _e40 = tap(in_4, texel_3 + off, atlas, atlas_sampler, mark_atlas);
    return 0.5 * (_e38 + _e40);
}

struct fs_glyph_inkInput {
    metal::float2 texel [[user(loc0), center_perspective]];
    metal::float2 uv_min [[user(loc1), flat]];
    metal::float2 uv_max [[user(loc2), flat]];
    metal::float4 fill [[user(loc3), flat]];
    uint sheet [[user(loc5), flat]];
    metal::float2 sheet_size [[user(loc6), flat]];
};
struct fs_glyph_inkOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_glyph_inkOutput fs_glyph_ink(
  fs_glyph_inkInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Locals& locals [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> atlas [[texture(0)]]
, metal::sampler atlas_sampler [[sampler(0)]]
, metal::texture2d<float, metal::access::sample> mark_atlas [[texture(1)]]
) {
    const VertexOut in = { position, varyings.texel, varyings.uv_min, varyings.uv_max, {}, varyings.fill, varyings.sheet, {}, varyings.sheet_size };
    float _e2 = coverage(in, in.texel, locals, atlas, atlas_sampler, mark_atlas);
    return fs_glyph_inkOutput { metal::float4(_e2, 0.0, 0.0, 0.0) };
}
