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
struct Locals {
    metal::float2 origin_points;
    metal::float2 viewport_points;
    float feather;
    float _pad;
    metal::float2 pitch_dir;
    metal::float2 depth_dir;
    metal::float2 _axis_pad;
    metal::float4 shadow;
    metal::float2 shadow_atlas_size;
    float shadow_falloff;
    float _shadow_pad;
};
struct VertexOut {
    metal::float4 position;
    metal::float2 local;
    metal::float2 half_extent;
    float shear;
    float outline_reach;
    float lead;
    float lead_fade;
    float lead_alpha;
    float cap_reach;
    char _pad9[8];
    metal::float4 core;
    metal::float4 outline;
    metal::float2 at;
    uint who;
    char _pad13[4];
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

float box_distance_trimmed(
    VertexOut in_1,
    float trim
) {
    float slope = in_1.shear;
    float skew = metal::sqrt(1.0 + (slope * slope));
    float across = (in_1.local.x - (slope * in_1.local.y)) / skew;
    float half_across = in_1.half_extent.x / skew;
    float along = in_1.local.y - (0.5 * trim);
    float half_along = in_1.half_extent.y - (0.5 * trim);
    metal::float2 q = metal::float2(metal::abs(across) - half_across, metal::abs(along) - half_along);
    return metal::min(metal::max(q.x, q.y), 0.0) + metal::length(metal::max(q, metal::float2(0.0)));
}

float box_distance(
    VertexOut in_2
) {
    float _e2 = box_distance_trimmed(in_2, 0.0);
    return _e2;
}

float inside(
    float d_1,
    float edge,
    constant Locals& locals
) {
    float _e4 = locals.feather;
    float f_1 = metal::max(_e4, 0.000001);
    return metal::clamp(((edge - d_1) / f_1) + 0.5, 0.0, 1.0);
}

float lead_coverage(
    VertexOut in_3,
    constant Locals& locals
) {
    float led = {};
    if (in_3.lead <= 0.0) {
        return 1.0;
    }
    float _e7 = locals.feather;
    float f_2 = metal::max(_e7, 0.000001);
    float u_1 = in_3.local.y + in_3.half_extent.y;
    led = in_3.lead_alpha;
    if (in_3.lead_fade > 0.0) {
        led = in_3.lead_alpha * metal::clamp(u_1 / metal::max(metal::min(in_3.lead_fade, in_3.lead), f_2), 0.0, 1.0);
    }
    float note = metal::clamp(((u_1 - in_3.lead) / f_2) + 0.5, 0.0, 1.0);
    float _e38 = led;
    return metal::mix(_e38, 1.0, note);
}

float outline_coverage(
    VertexOut in_4,
    float d_2,
    float reach,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_5 const& shadow_casters,
    constant Locals& locals,
    constant _mslBufferSizes& _buffer_sizes
) {
    bool local_1 = {};
    float full_1 = {};
    if (!((reach <= 0.0))) {
        float _e11 = locals.shadow.y;
        local_1 = _e11 <= 0.0;
    } else {
        local_1 = true;
    }
    bool _e15 = local_1;
    if (_e15) {
        return 0.0;
    }
    float _e20 = locals.shadow.x;
    float _e25 = locals.shadow_falloff;
    float _e26 = standoff_coverage(d_2, 2.0 * _e20, _e25);
    full_1 = _e26;
    float _e31 = locals.shadow.z;
    if (_e31 < 0.5) {
        float _e36 = shadow_kernel(in_4.who, in_4.at, shadow_atlas, shadow_sampler, shadow_casters, _buffer_sizes);
        full_1 = _e36;
    }
    float _e37 = full_1;
    float _e41 = locals.shadow.y;
    float _e43 = shadow_transmittance(_e37, _e41, 1.0);
    float _e46 = inside(d_2, reach, locals);
    return (1.0 - _e43) * _e46;
}

float cap_coverage(
    VertexOut in_5,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_5 const& shadow_casters,
    constant Locals& locals,
    constant _mslBufferSizes& _buffer_sizes
) {
    bool local_2 = {};
    float reach_1 = metal::min(in_5.cap_reach, in_5.outline_reach);
    if (!((in_5.lead <= 0.0))) {
        local_2 = reach_1 <= 0.0;
    } else {
        local_2 = true;
    }
    bool _e13 = local_2;
    if (_e13) {
        return 0.0;
    }
    float _e16 = box_distance_trimmed(in_5, in_5.lead);
    float _e17 = outline_coverage(in_5, _e16, reach_1, shadow_atlas, shadow_sampler, shadow_casters, locals, _buffer_sizes);
    float _e19 = inside(_e16, 0.0, locals);
    return _e17 * (1.0 - _e19);
}

metal::float4 outline_color(
    VertexOut in_6,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_5 const& shadow_casters,
    constant Locals& locals,
    constant _mslBufferSizes& _buffer_sizes
) {
    float _e1 = box_distance(in_6);
    float _e3 = outline_coverage(in_6, _e1, in_6.outline_reach, shadow_atlas, shadow_sampler, shadow_casters, locals, _buffer_sizes);
    float _e5 = inside(_e1, 0.0, locals);
    float _e9 = lead_coverage(in_6, locals);
    float wrap = (_e3 * (1.0 - _e5)) * _e9;
    float _e12 = cap_coverage(in_6, shadow_atlas, shadow_sampler, shadow_casters, locals, _buffer_sizes);
    return in_6.outline * metal::max(wrap, _e12);
}

metal::float3 linear_from_gamma_rgb(
    metal::float3 srgb
) {
    metal::bool3 cutoff = srgb < metal::float3(0.04045);
    metal::float3 lower = srgb / metal::float3(12.92);
    metal::float3 higher = metal::pow((srgb + metal::float3(0.055)) / metal::float3(1.055), metal::float3(2.4));
    return metal::select(higher, lower, cutoff);
}

struct fs_outline_linearInput {
    metal::float2 local [[user(loc0), center_perspective]];
    metal::float2 half_extent [[user(loc1), flat]];
    float shear [[user(loc2), flat]];
    float outline_reach [[user(loc3), flat]];
    float lead [[user(loc4), flat]];
    float lead_fade [[user(loc5), flat]];
    float lead_alpha [[user(loc6), flat]];
    float cap_reach [[user(loc7), flat]];
    metal::float4 core [[user(loc8), flat]];
    metal::float4 outline [[user(loc9), flat]];
    metal::float2 at [[user(loc10), center_perspective]];
    uint who [[user(loc11), flat]];
};
struct fs_outline_linearOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_outline_linearOutput fs_outline_linear(
  fs_outline_linearInput varyings [[stage_in]]
, metal::float4 position [[position]]
, metal::texture2d<float, metal::access::sample> shadow_atlas [[texture(0)]]
, metal::sampler shadow_sampler [[sampler(0)]]
, device type_5 const& shadow_casters [[buffer(1)]]
, constant Locals& locals [[buffer(0)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(2)]]
) {
    const VertexOut in = { position, varyings.local, varyings.half_extent, varyings.shear, varyings.outline_reach, varyings.lead, varyings.lead_fade, varyings.lead_alpha, varyings.cap_reach, {}, varyings.core, varyings.outline, varyings.at, varyings.who };
    metal::float4 _e1 = outline_color(in, shadow_atlas, shadow_sampler, shadow_casters, locals, _buffer_sizes);
    metal::float3 _e3 = linear_from_gamma_rgb(_e1.xyz);
    return fs_outline_linearOutput { metal::float4(_e3, _e1.w) };
}
