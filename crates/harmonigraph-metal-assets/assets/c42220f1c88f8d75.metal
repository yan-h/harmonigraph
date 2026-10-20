// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct Locals {
    metal::float4 shadow;
    metal::float2 origin_points;
    metal::float2 viewport_points;
    float feather;
    float _feather_pad;
    metal::float2 pitch_dir;
    metal::float2 depth_dir;
    metal::float2 origin_pixels;
    metal::float2 pixel_points;
    metal::float2 _tail_pad;
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
    metal::float4 aura;
    metal::float2 center;
    float feather;
    char _pad13[4];
    metal::float2 ramp;
    metal::float2 fade;
};
struct MaskOut {
    metal::float4 glow;
    float depth;
    char _pad2[12];
};
constant uint OCCLUDER_HEADER = 5u;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;
constant float GLOW_BASE = 0.8;

metal::float2 surface_point(
    VertexOut in_1,
    constant Locals& locals
) {
    metal::float2 _e3 = locals.origin_pixels;
    metal::float2 _e9 = locals.pixel_points;
    return (_e3 + in_1.position.xy) * _e9;
}

VertexOut on_screen(
    VertexOut in_2,
    constant Locals& locals
) {
    VertexOut out = {};
    out = in_2;
    metal::float2 _e2 = surface_point(in_2, locals);
    metal::float2 delta = _e2 - in_2.center;
    metal::float2 _e8 = locals.pitch_dir;
    metal::float2 _e12 = locals.depth_dir;
    out.local = metal::float2(metal::dot(delta, _e8), metal::dot(delta, _e12));
    VertexOut _e15 = out;
    return _e15;
}

float inside(
    VertexOut in_3,
    float d,
    float edge
) {
    float f = metal::max(in_3.feather, 0.000001);
    return metal::clamp(((edge - d) / f) + 0.5, 0.0, 1.0);
}

float outline_coverage(
    VertexOut in_4,
    float d_1,
    float reach,
    constant Locals& locals
) {
    bool local = {};
    bool local_1 = {};
    float sigma = locals.shadow.x;
    if (!((reach <= 0.0))) {
        float _e15 = locals.shadow.y;
        local = _e15 <= 0.0;
    } else {
        local = true;
    }
    bool _e19 = local;
    if (!(_e19)) {
        local_1 = sigma <= 0.0;
    } else {
        local_1 = true;
    }
    bool _e26 = local_1;
    if (_e26) {
        return 0.0;
    }
    float spread = locals.shadow.z;
    float x = metal::max(d_1 - spread, 0.0) / sigma;
    float _e39 = locals.shadow.w;
    float end = (_e39 - spread) / sigma;
    float tail = metal::exp((-0.5 * end) * end);
    float profile = metal::max(metal::exp((-0.5 * x) * x) - tail, 0.0) / metal::max(1.0 - tail, 0.000001);
    float _e61 = locals.shadow.y;
    float _e63 = inside(in_4, d_1, reach);
    return (_e61 * profile) * _e63;
}

float lead_coverage(
    VertexOut in_5
) {
    float led = {};
    float lead = in_5.lead.x;
    float lead_fade = in_5.lead.y;
    float lead_alpha = in_5.lead.z;
    if (lead <= 0.0) {
        return 1.0;
    }
    float f_1 = metal::max(in_5.feather, 0.000001);
    float u = in_5.local.y + in_5.half_extent.y;
    led = lead_alpha;
    if (lead_fade > 0.0) {
        led = lead_alpha * metal::clamp(u / metal::max(metal::min(lead_fade, lead), f_1), 0.0, 1.0);
    }
    float note = metal::clamp(((u - lead) / f_1) + 0.5, 0.0, 1.0);
    float _e35 = led;
    return metal::mix(_e35, 1.0, note);
}

float parallelogram_distance(
    VertexOut in_6,
    float trim,
    float half_pitch
) {
    metal::float2 p = {};
    metal::float2 w = {};
    float near = {};
    float within = {};
    metal::float2 v = {};
    float slope_1 = in_6.shear;
    float half_along = in_6.half_extent.y - (0.5 * trim);
    metal::float2 end_1 = metal::float2(slope_1 * half_along, half_along);
    p = in_6.local - ((0.5 * trim) * metal::float2(slope_1, 1.0));
    metal::float2 _e19 = p;
    metal::float2 _e20 = p;
    float _e23 = p.y;
    p = (_e23 < 0.0) ? -(_e20) : _e19;
    metal::float2 _e27 = p;
    w = _e27 - end_1;
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
    float side = (_e46 * end_1.y) - (_e50 * end_1.x);
    metal::float2 _e54 = p;
    metal::float2 _e55 = p;
    p = (side < 0.0) ? -(_e55) : _e54;
    metal::float2 _e60 = p;
    v = _e60 - metal::float2(half_pitch, 0.0);
    metal::float2 _e65 = v;
    metal::float2 _e66 = v;
    v = _e65 - (end_1 * metal::clamp(metal::dot(_e66, end_1) / metal::max(metal::dot(end_1, end_1), 0.000000000001), -1.0, 1.0));
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
    VertexOut in_7,
    float y
) {
    metal::float4 d_2 = in_7.taper_depth;
    metal::float4 w_1 = in_7.taper;
    if (y < in_7.taper_depth.y) {
        return metal::mix(in_7.taper.x, in_7.taper.y, metal::clamp((y - in_7.taper_depth.x) / metal::max(in_7.taper_depth.y - in_7.taper_depth.x, 0.000001), 0.0, 1.0));
    }
    if (y < in_7.taper_depth.z) {
        return metal::mix(in_7.taper.y, in_7.taper.z, metal::clamp((y - in_7.taper_depth.y) / metal::max(in_7.taper_depth.z - in_7.taper_depth.y, 0.000001), 0.0, 1.0));
    }
    return metal::mix(in_7.taper.z, in_7.taper.w, metal::clamp((y - in_7.taper_depth.z) / metal::max(in_7.taper_depth.w - in_7.taper_depth.z, 0.000001), 0.0, 1.0));
}

metal::float2 taper_vertex(
    VertexOut in_8,
    float depth,
    float width,
    float lo,
    float hi
) {
    bool local_2 = {};
    float y_1 = metal::clamp(depth, lo, hi);
    float _e6 = taper_at(in_8, y_1);
    if (depth >= lo) {
        local_2 = depth <= hi;
    } else {
        local_2 = false;
    }
    bool _e12 = local_2;
    float share = _e12 ? width : _e6;
    return metal::float2(y_1, in_8.half_extent.x * share);
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
    VertexOut in_9,
    float trim_1
) {
    float near_1 = {};
    bool local_3 = {};
    bool local_4 = {};
    float slope_2 = in_9.shear;
    float lo_1 = -(in_9.half_extent.y) + trim_1;
    float hi_1 = in_9.half_extent.y;
    metal::float2 p_3 = in_9.local;
    metal::float4 d_3 = in_9.taper_depth;
    metal::float4 w_2 = in_9.taper;
    float _e14 = taper_at(in_9, lo_1);
    metal::float2 start = metal::float2(lo_1, in_9.half_extent.x * _e14);
    metal::float2 _e19 = taper_vertex(in_9, in_9.taper_depth.x, in_9.taper.x, lo_1, hi_1);
    metal::float2 _e22 = taper_vertex(in_9, in_9.taper_depth.y, in_9.taper.y, lo_1, hi_1);
    metal::float2 _e25 = taper_vertex(in_9, in_9.taper_depth.z, in_9.taper.z, lo_1, hi_1);
    metal::float2 _e28 = taper_vertex(in_9, in_9.taper_depth.w, in_9.taper.w, lo_1, hi_1);
    float _e31 = taper_at(in_9, hi_1);
    metal::float2 end_2 = metal::float2(hi_1, in_9.half_extent.x * _e31);
    float _e42 = segment_distance2_(p_3, metal::float2((slope_2 * lo_1) - start.y, lo_1), metal::float2((slope_2 * lo_1) + start.y, lo_1));
    float _e51 = segment_distance2_(p_3, metal::float2((slope_2 * hi_1) - end_2.y, hi_1), metal::float2((slope_2 * hi_1) + end_2.y, hi_1));
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
    float _e67 = flanks_distance2_(p_3, slope_2, _e28, end_2);
    near_1 = metal::min(_e66, _e67);
    if (in_9.local.y > lo_1) {
        local_3 = in_9.local.y < hi_1;
    } else {
        local_3 = false;
    }
    bool _e76 = local_3;
    if (_e76) {
        float _e87 = taper_at(in_9, in_9.local.y);
        local_4 = metal::abs(in_9.local.x - (slope_2 * in_9.local.y)) < (in_9.half_extent.x * _e87);
    } else {
        local_4 = false;
    }
    bool inside_1 = local_4;
    float _e92 = near_1;
    float _e94 = near_1;
    return inside_1 ? -(metal::sqrt(_e94)) : metal::sqrt(_e92);
}

float box_distance_trimmed(
    VertexOut in_10,
    float trim_2
) {
    metal::float4 t = in_10.taper;
    if (metal::all(t == metal::float4(in_10.taper.x))) {
        float _e11 = parallelogram_distance(in_10, trim_2, in_10.half_extent.x * in_10.taper.x);
        return _e11;
    }
    float _e12 = tapered_distance(in_10, trim_2);
    return _e12;
}

float box_distance(
    VertexOut in_11
) {
    float _e2 = box_distance_trimmed(in_11, 0.0);
    return _e2;
}

float cap_coverage(
    VertexOut in_12,
    float held,
    constant Locals& locals
) {
    bool local_5 = {};
    float reach_1 = metal::min(in_12.lead.w, in_12.outline_reach);
    if (!((in_12.lead.x <= 0.0))) {
        local_5 = reach_1 <= 0.0;
    } else {
        local_5 = true;
    }
    bool _e16 = local_5;
    if (_e16) {
        return 0.0;
    }
    float _e20 = box_distance_trimmed(in_12, in_12.lead.x);
    float _e21 = outline_coverage(in_12, _e20, reach_1, locals);
    float _e23 = inside(in_12, _e20, 0.0);
    return _e21 * (1.0 - metal::max(_e23, held));
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

float body_held(
    VertexOut in_13,
    metal::texture2d<float, metal::access::sample> body_holdout
) {
    bool local_6 = {};
    metal::int2 texel = naga_f2i32(metal::floor(in_13.position.xy));
    metal::int2 size = static_cast<metal::int2>(metal::uint2(body_holdout.get_width(), body_holdout.get_height()));
    if (!(metal::any(texel < metal::int2(0)))) {
        local_6 = metal::any(texel >= size);
    } else {
        local_6 = true;
    }
    bool _e18 = local_6;
    if (_e18) {
        return 0.0;
    }
    uint clamped_lod_e22 = metal::min(uint(0), body_holdout.get_num_mip_levels() - 1);
    metal::float4 _e22 = body_holdout.read(metal::min(metal::uint2(texel), metal::uint2(body_holdout.get_width(clamped_lod_e22), body_holdout.get_height(clamped_lod_e22)) - 1), clamped_lod_e22);
    return _e22.x;
}

float along(
    VertexOut in_14,
    metal::float2 ends
) {
    float run = in_14.ramp.y - in_14.ramp.x;
    float t_1 = (metal::abs(run) > 0.000001) ? metal::clamp((in_14.local.y - in_14.ramp.x) / run, 0.0, 1.0) : 0.0;
    return metal::mix(ends.x, ends.y, t_1);
}

float outline_strength(
    VertexOut in_15,
    constant Locals& locals,
    metal::texture2d<float, metal::access::sample> body_holdout
) {
    float _e3 = taper_at(in_15, in_15.local.y);
    if (_e3 <= 0.0) {
        return 0.0;
    }
    float _e7 = box_distance(in_15);
    float _e8 = body_held(in_15, body_holdout);
    float _e10 = outline_coverage(in_15, _e7, in_15.outline_reach, locals);
    float _e12 = inside(in_15, _e7, 0.0);
    float _e17 = lead_coverage(in_15);
    float wrap = (_e10 * (1.0 - metal::max(_e12, _e8))) * _e17;
    float _e21 = cap_coverage(in_15, _e8, locals);
    float _e25 = along(in_15, in_15.fade);
    return (in_15.aura.w * metal::max(wrap, _e21)) * _e25;
}

struct fs_outline_maskInput {
    metal::float2 local [[user(loc0), center_perspective]];
    metal::float2 half_extent [[user(loc1), flat]];
    float shear [[user(loc2), flat]];
    float outline_reach [[user(loc3), flat]];
    metal::float4 lead [[user(loc4), flat]];
    metal::float4 taper_depth [[user(loc5), flat]];
    metal::float4 taper [[user(loc6), flat]];
    metal::float2 tremolo [[user(loc7), flat]];
    metal::float4 core [[user(loc8), flat]];
    metal::float4 aura [[user(loc9), flat]];
    metal::float2 center [[user(loc10), flat]];
    float feather [[user(loc12), flat]];
    metal::float2 ramp [[user(loc13), flat]];
    metal::float2 fade [[user(loc14), flat]];
};
struct fs_outline_maskOutput {
    metal::float4 glow [[color(0)]];
    float depth [[depth(any)]];
};
fragment fs_outline_maskOutput fs_outline_mask(
  fs_outline_maskInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Locals& locals [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> body_holdout [[texture(0)]]
) {
    const VertexOut in = { position, varyings.local, varyings.half_extent, varyings.shear, varyings.outline_reach, {}, varyings.lead, varyings.taper_depth, varyings.taper, varyings.tremolo, {}, varyings.core, varyings.aura, varyings.center, varyings.feather, {}, varyings.ramp, varyings.fade };
    VertexOut _e1 = on_screen(in, locals);
    float _e2 = outline_strength(_e1, locals, body_holdout);
    const auto _tmp = MaskOut {metal::float4(in.aura.xyz, _e2), 1.0 - _e2};
    return fs_outline_maskOutput { _tmp.glow, _tmp.depth };
}
