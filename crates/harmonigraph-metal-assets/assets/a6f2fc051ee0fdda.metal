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
typedef ShadowCaster type_5[1];
struct SplitOut {
    metal::float4 other;
    metal::float4 ink;
};
struct Locals {
    metal::float2 screen_points;
    metal::float2 atlas_size;
    metal::float2 mark_atlas_size;
    metal::float2 filter_axis;
    float pixels_per_point;
    float shadow_depth;
    metal::float2 shadow_atlas_size;
    float node_occlusion;
    float _pad0_;
    metal::float2 _pad1_;
};
struct BoxOut {
    metal::float4 position;
    metal::float2 at;
    float level;
    uint who;
};
constant float DISTANCE_KIND = 1.0;
constant float DISTANCE_COVERAGE_KIND = 2.0;
constant uint OCCLUDER_HEADER = 5u;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_STOP = 1.0;
constant float SHADOW_FALLOFF_MIN = -6.0;
constant float SHADOW_FALLOFF_MAX = 6.0;
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

float standoff_coverage(
    float d,
    float w,
    float falloff
) {
    float u = metal::clamp((metal::max(d, 0.0) / metal::max(w, 0.000001)) / SHADOW_STOP, 0.0, 1.0);
    float remaining = 1.0 - u;
    float shape = -(metal::clamp(falloff, SHADOW_FALLOFF_MIN, SHADOW_FALLOFF_MAX));
    if (metal::abs(shape) < 0.05) {
        return remaining * ((1.0 - ((shape * u) * 0.5)) + ((((shape * shape) * u) * ((2.0 * u) - 1.0)) / 12.0));
    }
    return (metal::exp(shape * remaining) - 1.0) / (metal::exp(shape) - 1.0);
}

float shadow_kernel(
    uint who,
    metal::float2 points,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_5 const& shadow_casters,
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

float local_shadow_transmittance(
    float full,
    float depth,
    float level
) {
    return 1.0 - ((metal::clamp(full, 0.0, 1.0) * metal::clamp(depth, 0.0, 1.0)) * metal::clamp(level, 0.0, 1.0));
}

struct fs_shadow_box_plainInput {
    metal::float2 at [[user(loc0), center_perspective]];
    float level [[user(loc1), flat]];
    uint who [[user(loc2), flat]];
};
struct fs_shadow_box_plainOutput {
    metal::float4 other [[color(0)]];
    metal::float4 ink [[color(1)]];
};
fragment fs_shadow_box_plainOutput fs_shadow_box_plain(
  fs_shadow_box_plainInput varyings [[stage_in]]
, metal::float4 position [[position]]
, metal::texture2d<float, metal::access::sample> shadow_atlas [[texture(4)]]
, metal::sampler shadow_sampler [[sampler(2)]]
, device type_5 const& shadow_casters [[buffer(1)]]
, constant Locals& locals [[buffer(0)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    const BoxOut in = { position, varyings.at, varyings.level, varyings.who };
    float _e3 = shadow_kernel(in.who, in.at, shadow_atlas, shadow_sampler, shadow_casters, _buffer_sizes);
    float _e6 = locals.shadow_depth;
    float _e8 = local_shadow_transmittance(_e3, _e6, in.level);
    metal::float4 shadow = metal::float4(0.0, 0.0, 0.0, 1.0 - _e8);
    const auto _tmp = SplitOut {shadow, shadow};
    return fs_shadow_box_plainOutput { _tmp.other, _tmp.ink };
}
