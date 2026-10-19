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
struct DotShadowLocals {
    metal::float2 screen_points;
    metal::float2 shadow_atlas_size;
    metal::float4 shadow;
};
struct DotShadowOut {
    metal::float4 position;
    metal::float2 local;
    float radius;
    char _pad3[4];
    metal::float2 at;
    uint who;
    float spread;
};
constant uint OCCLUDER_HEADER = 5u;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;
constant float GLOW_BASE = 0.8;

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

metal::float2 shadow_profile(
    uint who,
    metal::float2 points,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_5 const& shadow_casters,
    constant _mslBufferSizes& _buffer_sizes
) {
    if (who >= (1 + (_buffer_sizes.size2 - 0 - 64) / 64)) {
        return metal::float2(0.0);
    }
    metal::float4 cell_1 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size2 - 0 - 64) / 64)].cell;
    bool _e11 = cell_packed(cell_1);
    if (!(_e11)) {
        return metal::float2(0.0);
    }
    metal::float2 atlas = static_cast<metal::float2>(metal::uint2(shadow_atlas.get_width(), shadow_atlas.get_height()));
    metal::float4 map = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size2 - 0 - 64) / 64)].map;
    metal::float2 texel = metal::clamp(map.xy + (points * map.z), cell_1.xy + metal::float2(0.5), (cell_1.xy + cell_1.zw) - metal::float2(0.5));
    metal::float4 _e41 = shadow_atlas.sample(shadow_sampler, texel / atlas, metal::level(0.0));
    metal::float2 held = _e41.xy;
    return metal::float2(metal::min(GAUSSIAN_GAIN * metal::clamp(held.x, 0.0, 1.0), 1.0), held.y);
}

float shadow_kernel(
    uint who_1,
    metal::float2 points_1,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_5 const& shadow_casters,
    constant _mslBufferSizes& _buffer_sizes
) {
    metal::float2 _e2 = shadow_profile(who_1, points_1, shadow_atlas, shadow_sampler, shadow_casters, _buffer_sizes);
    return _e2.x;
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

struct fs_dot_shadowInput {
    metal::float2 local [[user(loc0), center_perspective]];
    float radius [[user(loc1), flat]];
    metal::float2 at [[user(loc2), center_perspective]];
    uint who [[user(loc3), flat]];
    float spread [[user(loc4), flat]];
};
struct fs_dot_shadowOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_dot_shadowOutput fs_dot_shadow(
  fs_dot_shadowInput varyings [[stage_in]]
, metal::float4 position [[position]]
, metal::texture2d<float, metal::access::sample> shadow_atlas [[texture(0)]]
, metal::sampler shadow_sampler [[sampler(0)]]
, device type_5 const& shadow_casters [[buffer(1)]]
, constant DotShadowLocals& dot_locals [[buffer(0)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    const DotShadowOut in = { position, varyings.local, varyings.radius, {}, varyings.at, varyings.who, varyings.spread };
    float _e3 = shadow_kernel(in.who, in.at, shadow_atlas, shadow_sampler, shadow_casters, _buffer_sizes);
    float level_1 = shadow_casters[metal::min(unsigned(in.who), (_buffer_sizes.size2 - 0 - 64) / 64)].shade.x;
    float _e13 = dot_locals.shadow.y;
    float _e14 = shadow_transmittance(_e3, _e13, level_1);
    float alpha = 1.0 - _e14;
    return fs_dot_shadowOutput { metal::float4(0.0, 0.0, 0.0, alpha) };
}
