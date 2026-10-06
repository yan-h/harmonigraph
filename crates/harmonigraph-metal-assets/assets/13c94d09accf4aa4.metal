// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct Locals {
    metal::float2 origin_points;
    metal::float2 viewport_points;
    float feather;
    float _feather_pad;
    metal::float2 pitch_dir;
    metal::float2 depth_dir;
    metal::float2 holdout_origin;
    metal::float4 shadow;
    metal::float2 shadow_atlas_size;
    float shadow_falloff;
    float _shadow_pad;
    metal::float2 origin_pixels;
    metal::float2 pixel_points;
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
    metal::float2 tremolo;
    char _pad9[8];
    metal::float4 core;
    metal::float4 outline;
    metal::float2 center;
    uint who;
    float feather;
    metal::float2 ramp;
    metal::float2 fade;
};
constant float DISTANCE_KIND = 1.0;
constant float DISTANCE_COVERAGE_KIND = 2.0;
constant uint OCCLUDER_HEADER = 5u;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_STOP = 1.0;
constant float SHADOW_FALLOFF_MIN = -6.0;
constant float SHADOW_FALLOFF_MAX = 6.0;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;
constant float GLOW_BASE = 0.8;

float taper_at(
    VertexOut in_1,
    float y
) {
    metal::float4 d_1 = in_1.taper_depth;
    metal::float4 w_1 = in_1.taper;
    if (y < in_1.taper_depth.y) {
        return metal::mix(in_1.taper.x, in_1.taper.y, metal::clamp((y - in_1.taper_depth.x) / metal::max(in_1.taper_depth.y - in_1.taper_depth.x, 0.000001), 0.0, 1.0));
    }
    if (y < in_1.taper_depth.z) {
        return metal::mix(in_1.taper.y, in_1.taper.z, metal::clamp((y - in_1.taper_depth.y) / metal::max(in_1.taper_depth.z - in_1.taper_depth.y, 0.000001), 0.0, 1.0));
    }
    return metal::mix(in_1.taper.z, in_1.taper.w, metal::clamp((y - in_1.taper_depth.z) / metal::max(in_1.taper_depth.w - in_1.taper_depth.z, 0.000001), 0.0, 1.0));
}

float parallelogram_distance(
    VertexOut in_2,
    float trim,
    float half_pitch
) {
    metal::float2 p = {};
    metal::float2 w = {};
    float near = {};
    float within = {};
    metal::float2 v = {};
    float slope_1 = in_2.shear;
    float half_along = in_2.half_extent.y - (0.5 * trim);
    metal::float2 end = metal::float2(slope_1 * half_along, half_along);
    p = in_2.local - ((0.5 * trim) * metal::float2(slope_1, 1.0));
    metal::float2 _e19 = p;
    metal::float2 _e20 = p;
    float _e23 = p.y;
    p = (_e23 < 0.0) ? -(_e20) : _e19;
    metal::float2 _e27 = p;
    w = _e27 - end;
    float _e31 = w.x;
    float _e33 = w.x;
    w.x = _e31 - metal::clamp(_e33, -(half_pitch), half_pitch);
    metal::float2 _e37 = w;
    metal::float2 _e38 = w;
    near = metal::dot(_e37, _e38);
    float _e42 = w.y;
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

metal::float2 taper_vertex(
    VertexOut in_3,
    float depth,
    float width,
    float lo,
    float hi
) {
    bool local = {};
    float y_1 = metal::clamp(depth, lo, hi);
    float _e6 = taper_at(in_3, y_1);
    if (depth >= lo) {
        local = depth <= hi;
    } else {
        local = false;
    }
    bool _e12 = local;
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
    bool local_1 = {};
    bool local_2 = {};
    float slope_2 = in_4.shear;
    float lo_1 = -(in_4.half_extent.y) + trim_1;
    float hi_1 = in_4.half_extent.y;
    metal::float2 p_3 = in_4.local;
    metal::float4 d_2 = in_4.taper_depth;
    metal::float4 w_2 = in_4.taper;
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
        local_1 = in_4.local.y < hi_1;
    } else {
        local_1 = false;
    }
    bool _e76 = local_1;
    if (_e76) {
        float _e87 = taper_at(in_4, in_4.local.y);
        local_2 = metal::abs(in_4.local.x - (slope_2 * in_4.local.y)) < (in_4.half_extent.x * _e87);
    } else {
        local_2 = false;
    }
    bool inside_1 = local_2;
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
    float d,
    float edge
) {
    float f = metal::max(in_7.feather, 0.000001);
    return metal::clamp(((edge - d) / f) + 0.5, 0.0, 1.0);
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
    float u = in_8.local.y + in_8.half_extent.y;
    led = lead_alpha;
    if (lead_fade > 0.0) {
        led = lead_alpha * metal::clamp(u / metal::max(metal::min(lead_fade, lead), f_1), 0.0, 1.0);
    }
    float note = metal::clamp(((u - lead) / f_1) + 0.5, 0.0, 1.0);
    float _e35 = led;
    return metal::mix(_e35, 1.0, note);
}

metal::float2 surface_point(
    VertexOut in_9,
    constant Locals& locals
) {
    metal::float2 _e3 = locals.origin_pixels;
    metal::float2 _e9 = locals.pixel_points;
    return (_e3 + in_9.position.xy) * _e9;
}

VertexOut on_screen(
    VertexOut in_10,
    constant Locals& locals
) {
    VertexOut out = {};
    out = in_10;
    metal::float2 _e2 = surface_point(in_10, locals);
    metal::float2 delta = _e2 - in_10.center;
    metal::float2 _e8 = locals.pitch_dir;
    metal::float2 _e12 = locals.depth_dir;
    out.local = metal::float2(metal::dot(delta, _e8), metal::dot(delta, _e12));
    VertexOut _e15 = out;
    return _e15;
}

float along(
    VertexOut in_11,
    metal::float2 ends
) {
    float run = in_11.ramp.y - in_11.ramp.x;
    float t_1 = (metal::abs(run) > 0.000001) ? metal::clamp((in_11.local.y - in_11.ramp.x) / run, 0.0, 1.0) : 0.0;
    return metal::mix(ends.x, ends.y, t_1);
}

float tremolo_tone(
    VertexOut in_12
) {
    bool local_3 = {};
    float scale = in_12.tremolo.y;
    if (!((scale <= 0.0))) {
        local_3 = in_12.local.y < (-(in_12.half_extent.y) + in_12.lead.x);
    } else {
        local_3 = true;
    }
    bool _e18 = local_3;
    if (_e18) {
        return 1.0;
    }
    float x = (in_12.local.x - (in_12.shear * in_12.local.y)) / scale;
    float _e32 = taper_at(in_12, in_12.local.y);
    float half_width = (in_12.half_extent.x * _e32) / scale;
    float reach = metal::min(4.0, half_width * 0.6666667);
    float diagonal = (in_12.tremolo.x - (in_12.local.y / scale)) - (0.7 * x);
    float stroke_distance = (metal::fract((diagonal / 6.0) + 0.5) - 0.5) * 6.0;
    float distance = metal::max(metal::abs(x) - reach, (metal::abs(stroke_distance) / 1.220656) - 0.75);
    float _e67 = inside(in_12, distance * scale, 0.0);
    float inset = metal::clamp((half_width - metal::abs(x)) / metal::max(half_width * 0.5, 0.000001), 0.0, 1.0);
    return 1.0 - ((0.75 * _e67) * inset);
}

metal::float4 core_color(
    VertexOut in_13
) {
    float _e3 = taper_at(in_13, in_13.local.y);
    if (_e3 <= 0.0) {
        return metal::float4(0.0);
    }
    float _e10 = tremolo_tone(in_13);
    metal::float4 color = metal::float4(in_13.core.xyz * _e10, in_13.core.w);
    float _e15 = box_distance(in_13);
    float _e17 = inside(in_13, _e15, 0.0);
    float _e19 = lead_coverage(in_13);
    float _e22 = along(in_13, in_13.fade);
    return ((color * _e17) * _e19) * _e22;
}

struct fs_core_gammaInput {
    metal::float2 local [[user(loc0), center_perspective]];
    metal::float2 half_extent [[user(loc1), flat]];
    float shear [[user(loc2), flat]];
    float outline_reach [[user(loc3), flat]];
    metal::float4 lead [[user(loc4), flat]];
    metal::float4 taper_depth [[user(loc5), flat]];
    metal::float4 taper [[user(loc6), flat]];
    metal::float2 tremolo [[user(loc7), flat]];
    metal::float4 core [[user(loc8), flat]];
    metal::float4 outline [[user(loc9), flat]];
    metal::float2 center [[user(loc10), flat]];
    uint who [[user(loc11), flat]];
    float feather [[user(loc12), flat]];
    metal::float2 ramp [[user(loc13), flat]];
    metal::float2 fade [[user(loc14), flat]];
};
struct fs_core_gammaOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_core_gammaOutput fs_core_gamma(
  fs_core_gammaInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Locals& locals [[buffer(0)]]
) {
    const VertexOut in = { position, varyings.local, varyings.half_extent, varyings.shear, varyings.outline_reach, {}, varyings.lead, varyings.taper_depth, varyings.taper, varyings.tremolo, {}, varyings.core, varyings.outline, varyings.center, varyings.who, varyings.feather, varyings.ramp, varyings.fade };
    VertexOut _e1 = on_screen(in, locals);
    metal::float4 _e2 = core_color(_e1);
    return fs_core_gammaOutput { _e2 };
}
