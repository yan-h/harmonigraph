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
struct DotShadowLocals {
    metal::float2 screen_points;
    metal::float2 shadow_atlas_size;
    metal::float4 shadow;
    metal::float4 falloff;
};
struct DotShadowOut {
    metal::float4 position;
    metal::float2 local;
    float radius;
    char _pad3[4];
    metal::float2 at;
    uint who;
    char _pad5[4];
};
constant float DISTANCE_KIND = 1.0;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_FALLOFF_FLOOR = 0.35;
constant float SHADOW_TAIL = 4.0;
constant float SHADOW_FALLOFF_FREE = 0.64025325;
constant float SHADOW_STOP = 2.0;
constant float SHADOW_INVISIBLE_FOLDS = 1.5586027;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;

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
    float full_1,
    float depth,
    float level
) {
    float keep = metal::max(1.0 - metal::clamp(depth, 0.0, 1.0), SHADOW_KEEP_FLOOR);
    float through = metal::pow(keep, metal::clamp(full_1, 0.0, 1.0));
    return 1.0 - (metal::clamp(level, 0.0, 1.0) * (1.0 - through));
}

float dot_distance(
    DotShadowOut in_1
) {
    return metal::length(in_1.local) - in_1.radius;
}

struct fs_dot_shadowInput {
    metal::float2 local [[user(loc0), center_perspective]];
    float radius [[user(loc1), flat]];
    metal::float2 at [[user(loc2), center_perspective]];
    uint who [[user(loc3), flat]];
};
struct fs_dot_shadowOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_dot_shadowOutput fs_dot_shadow(
  fs_dot_shadowInput varyings [[stage_in]]
, metal::float4 position [[position]]
, metal::texture2d<float, metal::access::sample> shadow_atlas [[texture(0)]]
, metal::sampler shadow_sampler [[sampler(0)]]
, device type_4 const& shadow_casters [[buffer(1)]]
, constant DotShadowLocals& dot_locals [[buffer(0)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(2)]]
) {
    const DotShadowOut in = { position, varyings.local, varyings.radius, {}, varyings.at, varyings.who };
    float full = {};
    float _e1 = dot_distance(in);
    float _e5 = dot_locals.shadow.x;
    float _e11 = dot_locals.falloff.x;
    float _e12 = standoff_coverage(_e1, 2.0 * _e5, _e11);
    full = _e12;
    float _e17 = dot_locals.shadow.z;
    if (_e17 < 0.5) {
        float _e22 = shadow_kernel(in.who, in.at, shadow_atlas, shadow_sampler, shadow_casters, _buffer_sizes);
        full = _e22;
    }
    float level_1 = shadow_casters[metal::min(unsigned(in.who), (_buffer_sizes.size2 - 0 - 64) / 64)].shade.x;
    float _e29 = full;
    float _e33 = dot_locals.shadow.y;
    float _e34 = shadow_transmittance(_e29, _e33, level_1);
    float alpha = 1.0 - _e34;
    return fs_dot_shadowOutput { metal::float4(0.0, 0.0, 0.0, alpha) };
}
