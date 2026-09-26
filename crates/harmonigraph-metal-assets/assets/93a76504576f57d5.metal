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
    float light;
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
    char _pad5[8];
    metal::float4 lead;
    metal::float4 taper_depth;
    metal::float4 taper;
    metal::float4 core;
    metal::float4 outline;
    metal::float2 at;
    uint who;
    float feather;
    metal::float2 ramp;
    char _pad14[8];
    metal::float4 reads;
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

float parallelogram_distance(
    VertexOut in_1,
    float trim,
    float half_pitch
) {
    metal::float2 p = {};
    metal::float2 w_1 = {};
    float near = {};
    float within = {};
    metal::float2 v = {};
    float slope_1 = in_1.shear;
    float half_along = in_1.half_extent.y - (0.5 * trim);
    metal::float2 end = metal::float2(slope_1 * half_along, half_along);
    p = in_1.local - ((0.5 * trim) * metal::float2(slope_1, 1.0));
    metal::float2 _e19 = p;
    metal::float2 _e20 = p;
    float _e23 = p.y;
    p = (_e23 < 0.0) ? -(_e20) : _e19;
    metal::float2 _e27 = p;
    w_1 = _e27 - end;
    float _e31 = w_1.x;
    float _e33 = w_1.x;
    w_1.x = _e31 - metal::clamp(_e33, -(half_pitch), half_pitch);
    metal::float2 _e37 = w_1;
    metal::float2 _e38 = w_1;
    near = metal::dot(_e37, _e38);
    float _e42 = w_1.y;
    within = -(_e42);
    float _e46 = p.x;
    float _e50 = p.y;
    float side = (_e46 * end.y) - (_e50 * end.x);
    metal::float2 _e54 = p;
    metal::float2 _e55 = p;
    p = (side < 0.0) ? -(_e55) : _e54;
    metal::float2 _e60 = p;
    v = _e60 - metal::float2(half_pitch, 0.0);
    metal::float2 _e65 = v;
    metal::float2 _e66 = v;
    v = _e65 - (end * metal::clamp(metal::dot(_e66, end) / metal::max(metal::dot(end, end), 0.000000000001), -1.0, 1.0));
    float _e77 = near;
    metal::float2 _e78 = v;
    metal::float2 _e79 = v;
    near = metal::min(_e77, metal::dot(_e78, _e79));
    float _e82 = within;
    within = metal::min(_e82, (half_pitch * half_along) - metal::abs(side));
    float _e87 = near;
    float _e89 = near;
    float _e92 = within;
    return (_e92 > 0.0) ? -(metal::sqrt(_e89)) : metal::sqrt(_e87);
}

float taper_at(
    VertexOut in_2,
    float y
) {
    metal::float4 d_3 = in_2.taper_depth;
    metal::float4 w_2 = in_2.taper;
    if (y < in_2.taper_depth.y) {
        return metal::mix(in_2.taper.x, in_2.taper.y, metal::clamp((y - in_2.taper_depth.x) / metal::max(in_2.taper_depth.y - in_2.taper_depth.x, 0.000001), 0.0, 1.0));
    }
    if (y < in_2.taper_depth.z) {
        return metal::mix(in_2.taper.y, in_2.taper.z, metal::clamp((y - in_2.taper_depth.y) / metal::max(in_2.taper_depth.z - in_2.taper_depth.y, 0.000001), 0.0, 1.0));
    }
    return metal::mix(in_2.taper.z, in_2.taper.w, metal::clamp((y - in_2.taper_depth.z) / metal::max(in_2.taper_depth.w - in_2.taper_depth.z, 0.000001), 0.0, 1.0));
}

metal::float2 taper_vertex(
    VertexOut in_3,
    float depth_1,
    float width,
    float lo,
    float hi
) {
    bool local_1 = {};
    float y_1 = metal::clamp(depth_1, lo, hi);
    float _e6 = taper_at(in_3, y_1);
    if (depth_1 >= lo) {
        local_1 = depth_1 <= hi;
    } else {
        local_1 = false;
    }
    bool _e12 = local_1;
    float share = _e12 ? width : _e6;
    return metal::float2(y_1, in_3.half_extent.x * share);
}

float segment_distance2_(
    metal::float2 p_1,
    metal::float2 a,
    metal::float2 b
) {
    metal::float2 pa = p_1 - a;
    metal::float2 ba = b - a;
    float h = metal::clamp(metal::dot(pa, ba) / metal::max(metal::dot(ba, ba), 0.000000000001), 0.0, 1.0);
    metal::float2 q = pa - (ba * h);
    return metal::dot(q, q);
}

float flanks_distance2_(
    metal::float2 p_2,
    float slope,
    metal::float2 a_1,
    metal::float2 b_1
) {
    metal::float2 near_2 = metal::float2(slope * a_1.x, a_1.x);
    metal::float2 far = metal::float2(slope * b_1.x, b_1.x);
    metal::float2 across_a = metal::float2(a_1.y, 0.0);
    metal::float2 across_b = metal::float2(b_1.y, 0.0);
    float _e20 = segment_distance2_(p_2, near_2 + across_a, far + across_b);
    float _e23 = segment_distance2_(p_2, near_2 - across_a, far - across_b);
    return metal::min(_e20, _e23);
}

float tapered_distance(
    VertexOut in_4,
    float trim_1
) {
    float near_1 = {};
    bool local_2 = {};
    bool local_3 = {};
    float slope_2 = in_4.shear;
    float lo_1 = -(in_4.half_extent.y) + trim_1;
    float hi_1 = in_4.half_extent.y;
    metal::float2 p_3 = in_4.local;
    metal::float4 d_4 = in_4.taper_depth;
    metal::float4 w_3 = in_4.taper;
    float _e14 = taper_at(in_4, lo_1);
    metal::float2 start = metal::float2(lo_1, in_4.half_extent.x * _e14);
    metal::float2 _e19 = taper_vertex(in_4, in_4.taper_depth.x, in_4.taper.x, lo_1, hi_1);
    metal::float2 _e22 = taper_vertex(in_4, in_4.taper_depth.y, in_4.taper.y, lo_1, hi_1);
    metal::float2 _e25 = taper_vertex(in_4, in_4.taper_depth.z, in_4.taper.z, lo_1, hi_1);
    metal::float2 _e28 = taper_vertex(in_4, in_4.taper_depth.w, in_4.taper.w, lo_1, hi_1);
    float _e31 = taper_at(in_4, hi_1);
    metal::float2 end_1 = metal::float2(hi_1, in_4.half_extent.x * _e31);
    float _e42 = segment_distance2_(p_3, metal::float2((slope_2 * lo_1) - start.y, lo_1), metal::float2((slope_2 * lo_1) + start.y, lo_1));
    float _e51 = segment_distance2_(p_3, metal::float2((slope_2 * hi_1) - end_1.y, hi_1), metal::float2((slope_2 * hi_1) + end_1.y, hi_1));
    near_1 = metal::min(_e42, _e51);
    float _e54 = near_1;
    float _e55 = flanks_distance2_(p_3, slope_2, start, _e19);
    near_1 = metal::min(_e54, _e55);
    float _e57 = near_1;
    float _e58 = flanks_distance2_(p_3, slope_2, _e19, _e22);
    near_1 = metal::min(_e57, _e58);
    float _e60 = near_1;
    float _e61 = flanks_distance2_(p_3, slope_2, _e22, _e25);
    near_1 = metal::min(_e60, _e61);
    float _e63 = near_1;
    float _e64 = flanks_distance2_(p_3, slope_2, _e25, _e28);
    near_1 = metal::min(_e63, _e64);
    float _e66 = near_1;
    float _e67 = flanks_distance2_(p_3, slope_2, _e28, end_1);
    near_1 = metal::min(_e66, _e67);
    if (in_4.local.y > lo_1) {
        local_2 = in_4.local.y < hi_1;
    } else {
        local_2 = false;
    }
    bool _e76 = local_2;
    if (_e76) {
        float _e87 = taper_at(in_4, in_4.local.y);
        local_3 = metal::abs(in_4.local.x - (slope_2 * in_4.local.y)) < (in_4.half_extent.x * _e87);
    } else {
        local_3 = false;
    }
    bool inside_1 = local_3;
    float _e92 = near_1;
    float _e94 = near_1;
    return inside_1 ? -(metal::sqrt(_e94)) : metal::sqrt(_e92);
}

float box_distance_trimmed(
    VertexOut in_5,
    float trim_2
) {
    metal::float4 t = in_5.taper;
    if (metal::all(t == metal::float4(in_5.taper.x))) {
        float _e11 = parallelogram_distance(in_5, trim_2, in_5.half_extent.x * in_5.taper.x);
        return _e11;
    }
    float _e12 = tapered_distance(in_5, trim_2);
    return _e12;
}

float box_distance(
    VertexOut in_6
) {
    float _e2 = box_distance_trimmed(in_6, 0.0);
    return _e2;
}

float inside(
    VertexOut in_7,
    float d_1,
    float edge
) {
    float f = metal::max(in_7.feather, 0.000001);
    return metal::clamp(((edge - d_1) / f) + 0.5, 0.0, 1.0);
}

float lead_coverage(
    VertexOut in_8
) {
    float led = {};
    float lead = in_8.lead.x;
    float lead_fade = in_8.lead.y;
    float lead_alpha = in_8.lead.z;
    if (lead <= 0.0) {
        return 1.0;
    }
    float f_1 = metal::max(in_8.feather, 0.000001);
    float u_1 = in_8.local.y + in_8.half_extent.y;
    led = lead_alpha;
    if (lead_fade > 0.0) {
        led = lead_alpha * metal::clamp(u_1 / metal::max(metal::min(lead_fade, lead), f_1), 0.0, 1.0);
    }
    float note = metal::clamp(((u_1 - lead) / f_1) + 0.5, 0.0, 1.0);
    float _e35 = led;
    return metal::mix(_e35, 1.0, note);
}

float outline_coverage(
    VertexOut in_9,
    float d_2,
    float reach,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_5 const& shadow_casters,
    constant Locals& locals,
    constant _mslBufferSizes& _buffer_sizes
) {
    bool local_4 = {};
    float full_1 = {};
    if (!((reach <= 0.0))) {
        float _e11 = locals.shadow.y;
        local_4 = _e11 <= 0.0;
    } else {
        local_4 = true;
    }
    bool _e15 = local_4;
    if (_e15) {
        return 0.0;
    }
    float _e20 = locals.shadow.x;
    float _e25 = locals.shadow_falloff;
    float _e26 = standoff_coverage(d_2, 2.0 * _e20, _e25);
    full_1 = _e26;
    float _e31 = locals.shadow.z;
    if (_e31 < 0.5) {
        float _e36 = shadow_kernel(in_9.who, in_9.at, shadow_atlas, shadow_sampler, shadow_casters, _buffer_sizes);
        full_1 = _e36;
    }
    float _e37 = full_1;
    float _e41 = locals.shadow.y;
    float _e43 = shadow_transmittance(_e37, _e41, 1.0);
    float _e46 = inside(in_9, d_2, reach);
    return (1.0 - _e43) * _e46;
}

float cap_coverage(
    VertexOut in_10,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_5 const& shadow_casters,
    constant Locals& locals,
    constant _mslBufferSizes& _buffer_sizes
) {
    bool local_5 = {};
    float reach_1 = metal::min(in_10.lead.w, in_10.outline_reach);
    if (!((in_10.lead.x <= 0.0))) {
        local_5 = reach_1 <= 0.0;
    } else {
        local_5 = true;
    }
    bool _e15 = local_5;
    if (_e15) {
        return 0.0;
    }
    float _e19 = box_distance_trimmed(in_10, in_10.lead.x);
    float _e20 = outline_coverage(in_10, _e19, reach_1, shadow_atlas, shadow_sampler, shadow_casters, locals, _buffer_sizes);
    float _e22 = inside(in_10, _e19, 0.0);
    return _e20 * (1.0 - _e22);
}

float along(
    VertexOut in_11,
    metal::float2 ends
) {
    float run = in_11.ramp.y - in_11.ramp.x;
    float t_1 = (metal::abs(run) > 0.000001) ? metal::clamp((in_11.local.y - in_11.ramp.x) / run, 0.0, 1.0) : 0.0;
    return metal::mix(ends.x, ends.y, t_1);
}

metal::float4 outline_color(
    VertexOut in_12,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_5 const& shadow_casters,
    constant Locals& locals,
    constant _mslBufferSizes& _buffer_sizes
) {
    float _e1 = box_distance(in_12);
    float _e3 = outline_coverage(in_12, _e1, in_12.outline_reach, shadow_atlas, shadow_sampler, shadow_casters, locals, _buffer_sizes);
    float _e5 = inside(in_12, _e1, 0.0);
    float _e9 = lead_coverage(in_12);
    float wrap = (_e3 * (1.0 - _e5)) * _e9;
    float _e12 = cap_coverage(in_12, shadow_atlas, shadow_sampler, shadow_casters, locals, _buffer_sizes);
    float _e17 = along(in_12, in_12.reads.xy);
    return (in_12.outline * metal::max(wrap, _e12)) * _e17;
}

struct fs_outline_gammaInput {
    metal::float2 local [[user(loc0), center_perspective]];
    metal::float2 half_extent [[user(loc1), flat]];
    float shear [[user(loc2), flat]];
    float outline_reach [[user(loc3), flat]];
    metal::float4 lead [[user(loc4), flat]];
    metal::float4 taper_depth [[user(loc5), flat]];
    metal::float4 taper [[user(loc6), flat]];
    metal::float4 core [[user(loc8), flat]];
    metal::float4 outline [[user(loc9), flat]];
    metal::float2 at [[user(loc10), center_perspective]];
    uint who [[user(loc11), flat]];
    float feather [[user(loc12), flat]];
    metal::float2 ramp [[user(loc13), flat]];
    metal::float4 reads [[user(loc14), flat]];
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
    const VertexOut in = { position, varyings.local, varyings.half_extent, varyings.shear, varyings.outline_reach, {}, varyings.lead, varyings.taper_depth, varyings.taper, varyings.core, varyings.outline, varyings.at, varyings.who, varyings.feather, varyings.ramp, {}, varyings.reads };
    metal::float4 _e1 = outline_color(in, shadow_atlas, shadow_sampler, shadow_casters, locals, _buffer_sizes);
    return fs_outline_gammaOutput { _e1 };
}
