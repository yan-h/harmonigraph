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
    float feather;
};
constant float DISTANCE_KIND = 1.0;
constant float DISTANCE_COVERAGE_KIND = 2.0;
constant uint OCCLUDER_HEADER = 5u;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_STOP = 1.0;
constant float SHADOW_FALLOFF_MIN = -6.0;
constant float SHADOW_FALLOFF_MAX = 6.0;
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
    metal::float2 p = {};
    metal::float2 w_1 = {};
    float near = {};
    float within = {};
    metal::float2 v = {};
    float slope = in_1.shear;
    float half_along = in_1.half_extent.y - (0.5 * trim);
    float half_pitch = in_1.half_extent.x;
    metal::float2 end = metal::float2(slope * half_along, half_along);
    p = in_1.local - ((0.5 * trim) * metal::float2(slope, 1.0));
    metal::float2 _e20 = p;
    metal::float2 _e21 = p;
    float _e24 = p.y;
    p = (_e24 < 0.0) ? -(_e21) : _e20;
    metal::float2 _e28 = p;
    w_1 = _e28 - end;
    float _e32 = w_1.x;
    float _e34 = w_1.x;
    w_1.x = _e32 - metal::clamp(_e34, -(half_pitch), half_pitch);
    metal::float2 _e38 = w_1;
    metal::float2 _e39 = w_1;
    near = metal::dot(_e38, _e39);
    float _e43 = w_1.y;
    within = -(_e43);
    float _e47 = p.x;
    float _e51 = p.y;
    float side = (_e47 * end.y) - (_e51 * end.x);
    metal::float2 _e55 = p;
    metal::float2 _e56 = p;
    p = (side < 0.0) ? -(_e56) : _e55;
    metal::float2 _e61 = p;
    v = _e61 - metal::float2(half_pitch, 0.0);
    metal::float2 _e66 = v;
    metal::float2 _e67 = v;
    v = _e66 - (end * metal::clamp(metal::dot(_e67, end) / metal::max(metal::dot(end, end), 0.000000000001), -1.0, 1.0));
    float _e78 = near;
    metal::float2 _e79 = v;
    metal::float2 _e80 = v;
    near = metal::min(_e78, metal::dot(_e79, _e80));
    float _e83 = within;
    within = metal::min(_e83, (half_pitch * half_along) - metal::abs(side));
    float _e88 = near;
    float _e90 = near;
    float _e93 = within;
    return (_e93 > 0.0) ? -(metal::sqrt(_e90)) : metal::sqrt(_e88);
}

float box_distance(
    VertexOut in_2
) {
    float _e2 = box_distance_trimmed(in_2, 0.0);
    return _e2;
}

float inside(
    VertexOut in_3,
    float d_1,
    float edge
) {
    float f = metal::max(in_3.feather, 0.000001);
    return metal::clamp(((edge - d_1) / f) + 0.5, 0.0, 1.0);
}

float lead_coverage(
    VertexOut in_4
) {
    float led = {};
    if (in_4.lead <= 0.0) {
        return 1.0;
    }
    float f_1 = metal::max(in_4.feather, 0.000001);
    float u_1 = in_4.local.y + in_4.half_extent.y;
    led = in_4.lead_alpha;
    if (in_4.lead_fade > 0.0) {
        led = in_4.lead_alpha * metal::clamp(u_1 / metal::max(metal::min(in_4.lead_fade, in_4.lead), f_1), 0.0, 1.0);
    }
    float note = metal::clamp(((u_1 - in_4.lead) / f_1) + 0.5, 0.0, 1.0);
    float _e36 = led;
    return metal::mix(_e36, 1.0, note);
}

float outline_coverage(
    VertexOut in_5,
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
        float _e36 = shadow_kernel(in_5.who, in_5.at, shadow_atlas, shadow_sampler, shadow_casters, _buffer_sizes);
        full_1 = _e36;
    }
    float _e37 = full_1;
    float _e41 = locals.shadow.y;
    float _e43 = shadow_transmittance(_e37, _e41, 1.0);
    float _e46 = inside(in_5, d_2, reach);
    return (1.0 - _e43) * _e46;
}

float cap_coverage(
    VertexOut in_6,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_5 const& shadow_casters,
    constant Locals& locals,
    constant _mslBufferSizes& _buffer_sizes
) {
    bool local_2 = {};
    float reach_1 = metal::min(in_6.cap_reach, in_6.outline_reach);
    if (!((in_6.lead <= 0.0))) {
        local_2 = reach_1 <= 0.0;
    } else {
        local_2 = true;
    }
    bool _e13 = local_2;
    if (_e13) {
        return 0.0;
    }
    float _e16 = box_distance_trimmed(in_6, in_6.lead);
    float _e17 = outline_coverage(in_6, _e16, reach_1, shadow_atlas, shadow_sampler, shadow_casters, locals, _buffer_sizes);
    float _e19 = inside(in_6, _e16, 0.0);
    return _e17 * (1.0 - _e19);
}

metal::float4 outline_color(
    VertexOut in_7,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_5 const& shadow_casters,
    constant Locals& locals,
    constant _mslBufferSizes& _buffer_sizes
) {
    float _e1 = box_distance(in_7);
    float _e3 = outline_coverage(in_7, _e1, in_7.outline_reach, shadow_atlas, shadow_sampler, shadow_casters, locals, _buffer_sizes);
    float _e5 = inside(in_7, _e1, 0.0);
    float _e9 = lead_coverage(in_7);
    float wrap = (_e3 * (1.0 - _e5)) * _e9;
    float _e12 = cap_coverage(in_7, shadow_atlas, shadow_sampler, shadow_casters, locals, _buffer_sizes);
    return in_7.outline * metal::max(wrap, _e12);
}

struct fs_outline_gammaInput {
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
    float feather [[user(loc12), flat]];
};
struct fs_outline_gammaOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_outline_gammaOutput fs_outline_gamma(
  fs_outline_gammaInput varyings [[stage_in]]
, metal::float4 position [[position]]
, metal::texture2d<float, metal::access::sample> shadow_atlas [[texture(0)]]
, metal::sampler shadow_sampler [[sampler(0)]]
, device type_5 const& shadow_casters [[buffer(1)]]
, constant Locals& locals [[buffer(0)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    const VertexOut in = { position, varyings.local, varyings.half_extent, varyings.shear, varyings.outline_reach, varyings.lead, varyings.lead_fade, varyings.lead_alpha, varyings.cap_reach, {}, varyings.core, varyings.outline, varyings.at, varyings.who, varyings.feather };
    metal::float4 _e1 = outline_color(in, shadow_atlas, shadow_sampler, shadow_casters, locals, _buffer_sizes);
    return fs_outline_gammaOutput { _e1 };
}
