// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct SurroundCut {
    metal::float4 cut;
};
struct Locals {
    SurroundCut surround;
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
    char _pad4[12];
    metal::float4 lead;
    metal::float4 taper_depth;
    metal::float4 taper;
    metal::float2 tremolo;
    char _pad8[8];
    metal::float4 core;
    metal::float4 aura;
    metal::float2 center;
    float feather;
    char _pad12[4];
    metal::float2 ramp;
    metal::float2 fade;
};
struct Rim {
    bool live;
    char _pad1[3];
    float d;
    float keep;
    float lead;
    float cap_d;
    float cap_keep;
};
struct SurroundOut {
    metal::float4 color;
    float depth;
    char _pad2[12];
};
constant uint OCCLUDER_HEADER = 5u;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;
constant float GLOW_BASE = 0.8;

float surround_ramp(
    float k,
    float x
) {
    float t = metal::clamp(x, 0.0, 1.0);
    return 1.0 - (t / ((k * (1.0 - t)) + 1.0));
}

float cut_reach(
    constant Locals& locals
) {
    float _e4 = locals.surround.cut.y;
    float _e9 = locals.surround.cut.x;
    return (_e9 > 0.0) ? _e4 : 0.0;
}

metal::float2 surface_point(
    VertexOut in,
    constant Locals& locals
) {
    metal::float2 _e3 = locals.origin_pixels;
    metal::float2 _e9 = locals.pixel_points;
    return (_e3 + in.position.xy) * _e9;
}

VertexOut on_screen(
    VertexOut in_1,
    constant Locals& locals
) {
    VertexOut out = {};
    out = in_1;
    metal::float2 _e2 = surface_point(in_1, locals);
    metal::float2 delta = _e2 - in_1.center;
    metal::float2 _e8 = locals.pitch_dir;
    metal::float2 _e12 = locals.depth_dir;
    out.local = metal::float2(metal::dot(delta, _e8), metal::dot(delta, _e12));
    VertexOut _e15 = out;
    return _e15;
}

float inside(
    VertexOut in_2,
    float d,
    float edge
) {
    float f = metal::max(in_2.feather, 0.000001);
    return metal::clamp(((edge - d) / f) + 0.5, 0.0, 1.0);
}

float cut_coverage(
    VertexOut in_3,
    float d_1,
    float reach,
    constant Locals& locals
) {
    bool local = {};
    float fade = locals.surround.cut.y;
    if (!((reach <= 0.0))) {
        local = fade <= 0.0;
    } else {
        local = true;
    }
    bool _e16 = local;
    if (_e16) {
        return 0.0;
    }
    float _e22 = locals.surround.cut.z;
    float _e26 = surround_ramp(_e22, metal::max(d_1, 0.0) / fade);
    float _e27 = inside(in_3, d_1, reach);
    return _e26 * _e27;
}

float lead_coverage(
    VertexOut in_4
) {
    float led = {};
    float lead = in_4.lead.x;
    float lead_fade = in_4.lead.y;
    float lead_alpha = in_4.lead.z;
    if (lead <= 0.0) {
        return 1.0;
    }
    float f_1 = metal::max(in_4.feather, 0.000001);
    float u = in_4.local.y + in_4.half_extent.y;
    led = lead_alpha;
    if (lead_fade > 0.0) {
        led = lead_alpha * metal::clamp(u / metal::max(metal::min(lead_fade, lead), f_1), 0.0, 1.0);
    }
    float note = metal::clamp(((u - lead) / f_1) + 0.5, 0.0, 1.0);
    float _e35 = led;
    return metal::mix(_e35, 1.0, note);
}

float parallelogram_distance(
    VertexOut in_5,
    float trim,
    float half_pitch
) {
    metal::float2 p = {};
    metal::float2 w = {};
    float near = {};
    float within = {};
    metal::float2 v = {};
    float slope_1 = in_5.shear;
    float half_along = in_5.half_extent.y - (0.5 * trim);
    metal::float2 end = metal::float2(slope_1 * half_along, half_along);
    p = in_5.local - ((0.5 * trim) * metal::float2(slope_1, 1.0));
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

float taper_at(
    VertexOut in_6,
    float y
) {
    metal::float4 d_2 = in_6.taper_depth;
    metal::float4 w_1 = in_6.taper;
    if (y < in_6.taper_depth.y) {
        return metal::mix(in_6.taper.x, in_6.taper.y, metal::clamp((y - in_6.taper_depth.x) / metal::max(in_6.taper_depth.y - in_6.taper_depth.x, 0.000001), 0.0, 1.0));
    }
    if (y < in_6.taper_depth.z) {
        return metal::mix(in_6.taper.y, in_6.taper.z, metal::clamp((y - in_6.taper_depth.y) / metal::max(in_6.taper_depth.z - in_6.taper_depth.y, 0.000001), 0.0, 1.0));
    }
    return metal::mix(in_6.taper.z, in_6.taper.w, metal::clamp((y - in_6.taper_depth.z) / metal::max(in_6.taper_depth.w - in_6.taper_depth.z, 0.000001), 0.0, 1.0));
}

metal::float2 taper_vertex(
    VertexOut in_7,
    float depth,
    float width,
    float lo,
    float hi
) {
    bool local_1 = {};
    float y_1 = metal::clamp(depth, lo, hi);
    float _e6 = taper_at(in_7, y_1);
    if (depth >= lo) {
        local_1 = depth <= hi;
    } else {
        local_1 = false;
    }
    bool _e12 = local_1;
    float share = _e12 ? width : _e6;
    return metal::float2(y_1, in_7.half_extent.x * share);
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
    VertexOut in_8,
    float trim_1
) {
    float near_1 = {};
    bool local_2 = {};
    bool local_3 = {};
    float slope_2 = in_8.shear;
    float lo_1 = -(in_8.half_extent.y) + trim_1;
    float hi_1 = in_8.half_extent.y;
    metal::float2 p_3 = in_8.local;
    metal::float4 d_3 = in_8.taper_depth;
    metal::float4 w_2 = in_8.taper;
    float _e14 = taper_at(in_8, lo_1);
    metal::float2 start = metal::float2(lo_1, in_8.half_extent.x * _e14);
    metal::float2 _e19 = taper_vertex(in_8, in_8.taper_depth.x, in_8.taper.x, lo_1, hi_1);
    metal::float2 _e22 = taper_vertex(in_8, in_8.taper_depth.y, in_8.taper.y, lo_1, hi_1);
    metal::float2 _e25 = taper_vertex(in_8, in_8.taper_depth.z, in_8.taper.z, lo_1, hi_1);
    metal::float2 _e28 = taper_vertex(in_8, in_8.taper_depth.w, in_8.taper.w, lo_1, hi_1);
    float _e31 = taper_at(in_8, hi_1);
    metal::float2 end_1 = metal::float2(hi_1, in_8.half_extent.x * _e31);
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
    if (in_8.local.y > lo_1) {
        local_2 = in_8.local.y < hi_1;
    } else {
        local_2 = false;
    }
    bool _e76 = local_2;
    if (_e76) {
        float _e87 = taper_at(in_8, in_8.local.y);
        local_3 = metal::abs(in_8.local.x - (slope_2 * in_8.local.y)) < (in_8.half_extent.x * _e87);
    } else {
        local_3 = false;
    }
    bool inside_1 = local_3;
    float _e92 = near_1;
    float _e94 = near_1;
    return inside_1 ? -(metal::sqrt(_e94)) : metal::sqrt(_e92);
}

float box_distance_trimmed(
    VertexOut in_9,
    float trim_2
) {
    metal::float4 t_1 = in_9.taper;
    if (metal::all(t_1 == metal::float4(in_9.taper.x))) {
        float _e11 = parallelogram_distance(in_9, trim_2, in_9.half_extent.x * in_9.taper.x);
        return _e11;
    }
    float _e12 = tapered_distance(in_9, trim_2);
    return _e12;
}

float box_distance(
    VertexOut in_10
) {
    float _e2 = box_distance_trimmed(in_10, 0.0);
    return _e2;
}

float cap_coverage(
    VertexOut in_11,
    Rim rim,
    constant Locals& locals
) {
    bool local_4 = {};
    float _e4 = cut_reach(locals);
    float reach_1 = metal::min(in_11.lead.w, _e4);
    if (!((in_11.lead.x <= 0.0))) {
        local_4 = reach_1 <= 0.0;
    } else {
        local_4 = true;
    }
    bool _e16 = local_4;
    if (_e16) {
        return 0.0;
    }
    float _e19 = cut_coverage(in_11, rim.cap_d, reach_1, locals);
    return _e19 * rim.cap_keep;
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

float body_held(
    VertexOut in_12,
    metal::texture2d<float, metal::access::sample> body_holdout
) {
    bool local_5 = {};
    metal::int2 texel = naga_f2i32(metal::floor(in_12.position.xy));
    metal::int2 size = static_cast<metal::int2>(metal::uint2(body_holdout.get_width(), body_holdout.get_height()));
    if (!(metal::any(texel < metal::int2(0)))) {
        local_5 = metal::any(texel >= size);
    } else {
        local_5 = true;
    }
    bool _e18 = local_5;
    if (_e18) {
        return 0.0;
    }
    uint clamped_lod_e22 = metal::min(uint(0), body_holdout.get_num_mip_levels() - 1);
    metal::float4 _e22 = body_holdout.read(metal::min(metal::uint2(texel), metal::uint2(body_holdout.get_width(clamped_lod_e22), body_holdout.get_height(clamped_lod_e22)) - 1), clamped_lod_e22);
    return _e22.x;
}

float along(
    VertexOut in_13,
    metal::float2 ends
) {
    float run = in_13.ramp.y - in_13.ramp.x;
    float t_2 = (metal::abs(run) > 0.000001) ? metal::clamp((in_13.local.y - in_13.ramp.x) / run, 0.0, 1.0) : 0.0;
    return metal::mix(ends.x, ends.y, t_2);
}

float cut_strength(
    VertexOut in_14,
    Rim rim_1,
    constant Locals& locals
) {
    if (!(rim_1.live)) {
        return 0.0;
    }
    float _e6 = cut_reach(locals);
    float _e7 = cut_coverage(in_14, rim_1.d, _e6, locals);
    float wrap = (_e7 * rim_1.keep) * rim_1.lead;
    float _e14 = cap_coverage(in_14, rim_1, locals);
    float _e18 = along(in_14, in_14.fade);
    return (in_14.aura.w * metal::max(wrap, _e14)) * _e18;
}

Rim rim_of(
    VertexOut in_15,
    metal::texture2d<float, metal::access::sample> body_holdout
) {
    Rim rim_2 = {};
    float _e5 = taper_at(in_15, in_15.local.y);
    rim_2.live = _e5 > 0.0;
    bool _e9 = rim_2.live;
    if (!(_e9)) {
        Rim _e11 = rim_2;
        return _e11;
    }
    float _e13 = box_distance(in_15);
    rim_2.d = _e13;
    float _e14 = body_held(in_15, body_holdout);
    float _e17 = rim_2.d;
    float _e19 = inside(in_15, _e17, 0.0);
    rim_2.keep = 1.0 - metal::max(_e19, _e14);
    float _e24 = lead_coverage(in_15);
    rim_2.lead = _e24;
    if (in_15.lead.x > 0.0) {
        float _e32 = box_distance_trimmed(in_15, in_15.lead.x);
        rim_2.cap_d = _e32;
        float _e35 = rim_2.cap_d;
        float _e37 = inside(in_15, _e35, 0.0);
        rim_2.cap_keep = 1.0 - metal::max(_e37, _e14);
    }
    Rim _e41 = rim_2;
    return _e41;
}

struct fs_note_surroundInput {
    metal::float2 local [[user(loc0), center_perspective]];
    metal::float2 half_extent [[user(loc1), flat]];
    float shear [[user(loc2), flat]];
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
struct fs_note_surroundOutput {
    metal::float4 color [[color(0)]];
    float depth [[depth(any)]];
};
fragment fs_note_surroundOutput fs_note_surround(
  fs_note_surroundInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Locals& locals [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> body_holdout [[texture(0)]]
) {
    const VertexOut fragment_ = { position, varyings.local, varyings.half_extent, varyings.shear, {}, varyings.lead, varyings.taper_depth, varyings.taper, varyings.tremolo, {}, varyings.core, varyings.aura, varyings.center, varyings.feather, {}, varyings.ramp, varyings.fade };
    VertexOut _e1 = on_screen(fragment_, locals);
    float _e6 = locals.surround.cut.x;
    Rim _e7 = rim_of(_e1, body_holdout);
    float _e8 = cut_strength(_e1, _e7, locals);
    float cut = _e6 * _e8;
    if (cut <= 0.0) {
        metal::discard_fragment();
    }
    const auto _tmp = SurroundOut {metal::float4(_e1.aura.xyz, cut), 1.0 - cut};
    return fs_note_surroundOutput { _tmp.color, _tmp.depth };
}
