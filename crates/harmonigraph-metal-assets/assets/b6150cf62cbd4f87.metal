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
typedef ShadowCaster type_4[1];
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
struct BoxOut {
    metal::float4 position;
    metal::float2 at;
    float level;
    uint who;
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

float shadow_stop(
    float falloff
) {
    if (falloff >= SHADOW_FALLOFF_FREE) {
        return SHADOW_STOP;
    }
    return metal::max(SHADOW_STOP, metal::pow(SHADOW_INVISIBLE_FOLDS, 1.0 / falloff));
}

float standoff_coverage(
    float d,
    float w,
    float falloff_1
) {
    float t = {};
    float u = metal::max(d, 0.0) / metal::max(w, 0.000001);
    float f = metal::max(falloff_1, SHADOW_FALLOFF_FLOOR);
    t = u;
    if (f != 1.0) {
        t = metal::pow(u, f);
    }
    float _e15 = t;
    float _e18 = shadow_stop(f);
    return metal::exp(-4.0 * _e15) * (1.0 - metal::smoothstep(1.0, _e18, u));
}

float shadow_kernel(
    uint who,
    metal::float2 points,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_4 const& shadow_casters,
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
    if (_e45 >= 0.5) {
        float _e52 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size2 - 0 - 64) / 64)].shade.z;
        float _e59 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size2 - 0 - 64) / 64)].shade.w;
        float _e60 = standoff_coverage(held, 2.0 * _e52, _e59);
        return metal::clamp(_e60, 0.0, 1.0);
    }
    return metal::min(GAUSSIAN_GAIN * metal::clamp(held, 0.0, 1.0), 1.0);
}

float shadow_transmittance(
    float full,
    float depth,
    float level
) {
    float keep = metal::max(1.0 - metal::clamp(depth, 0.0, 1.0), SHADOW_KEEP_FLOOR);
    float through = metal::pow(keep, metal::clamp(full, 0.0, 1.0));
    return 1.0 - (metal::clamp(level, 0.0, 1.0) * (1.0 - through));
}

struct fs_shadow_box_plainInput {
    metal::float2 at [[user(loc0), center_perspective]];
    float level [[user(loc1), flat]];
    uint who [[user(loc2), flat]];
};
struct fs_shadow_box_plainOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_shadow_box_plainOutput fs_shadow_box_plain(
  fs_shadow_box_plainInput varyings [[stage_in]]
, metal::float4 position [[position]]
, metal::texture2d<float, metal::access::sample> shadow_atlas [[texture(3)]]
, metal::sampler shadow_sampler [[sampler(1)]]
, device type_4 const& shadow_casters [[buffer(1)]]
, constant Locals& locals [[buffer(0)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(2)]]
) {
    const BoxOut in = { position, varyings.at, varyings.level, varyings.who };
    float _e3 = shadow_kernel(in.who, in.at, shadow_atlas, shadow_sampler, shadow_casters, _buffer_sizes);
    float _e6 = locals.shadow_depth;
    float _e8 = shadow_transmittance(_e3, _e6, in.level);
    return fs_shadow_box_plainOutput { metal::float4(0.0, 0.0, 0.0, 1.0 - _e8) };
}
