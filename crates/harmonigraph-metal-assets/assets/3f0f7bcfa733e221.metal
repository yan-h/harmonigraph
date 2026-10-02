// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint buffer_size15;
};

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
metal::float2 unpackFloat32x2_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7) {
    return metal::float2(as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0), as_type<float>(b7 << 24 | b6 << 16 | b5 << 8 | b4));
}
float unpackFloat32_(uint b0, uint b1, uint b2, uint b3) {
    return as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0);
}
metal::float4 unpackFloat32x4_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7, uint b8, uint b9, uint b10, uint b11, uint b12, uint b13, uint b14, uint b15) {
    return metal::float4(as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0), as_type<float>(b7 << 24 | b6 << 16 | b5 << 8 | b4), as_type<float>(b11 << 24 | b10 << 16 | b9 << 8 | b8), as_type<float>(b15 << 24 | b14 << 16 | b13 << 8 | b12));
}
metal::float4 unpackUnorm8x4_(metal::uchar b0, metal::uchar b1, metal::uchar b2, metal::uchar b3) {
    return metal::float4(float(b0) / 255.0f, float(b1) / 255.0f, float(b2) / 255.0f, float(b3) / 255.0f);
}

struct vs_noteOutput {
    metal::float4 position [[position]];
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
    metal::float2 fade [[user(loc14), flat]];
};
struct vb_15_type { metal::uchar data[100]; };
vertex vs_noteOutput vs_note(
  uint vertex_ [[vertex_id]]
, uint who [[instance_id]]
, constant Locals& locals [[buffer(0)]]
, const device vb_15_type* vb_15_in [[buffer(15)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(1)]]
) {
    metal::float2 center = {};
    metal::float2 half_extent = {};
    float shear = {};
    metal::float4 lead = {};
    metal::float4 core = {};
    metal::float4 outline = {};
    metal::float4 span_ramp = {};
    metal::float2 fade = {};
    metal::float4 taper_depth = {};
    metal::float4 taper = {};
    if (who < (_buffer_sizes.buffer_size15 / 100)) {
        const vb_15_type vb_15_elem = vb_15_in[who];
        center = unpackFloat32x2_(vb_15_elem.data[0], vb_15_elem.data[1], vb_15_elem.data[2], vb_15_elem.data[3], vb_15_elem.data[4], vb_15_elem.data[5], vb_15_elem.data[6], vb_15_elem.data[7]);
        half_extent = unpackFloat32x2_(vb_15_elem.data[8], vb_15_elem.data[9], vb_15_elem.data[10], vb_15_elem.data[11], vb_15_elem.data[12], vb_15_elem.data[13], vb_15_elem.data[14], vb_15_elem.data[15]);
        shear = unpackFloat32_(vb_15_elem.data[16], vb_15_elem.data[17], vb_15_elem.data[18], vb_15_elem.data[19]);
        lead = unpackFloat32x4_(vb_15_elem.data[20], vb_15_elem.data[21], vb_15_elem.data[22], vb_15_elem.data[23], vb_15_elem.data[24], vb_15_elem.data[25], vb_15_elem.data[26], vb_15_elem.data[27], vb_15_elem.data[28], vb_15_elem.data[29], vb_15_elem.data[30], vb_15_elem.data[31], vb_15_elem.data[32], vb_15_elem.data[33], vb_15_elem.data[34], vb_15_elem.data[35]);
        core = unpackUnorm8x4_(vb_15_elem.data[36], vb_15_elem.data[37], vb_15_elem.data[38], vb_15_elem.data[39]);
        outline = unpackUnorm8x4_(vb_15_elem.data[40], vb_15_elem.data[41], vb_15_elem.data[42], vb_15_elem.data[43]);
        span_ramp = unpackFloat32x4_(vb_15_elem.data[44], vb_15_elem.data[45], vb_15_elem.data[46], vb_15_elem.data[47], vb_15_elem.data[48], vb_15_elem.data[49], vb_15_elem.data[50], vb_15_elem.data[51], vb_15_elem.data[52], vb_15_elem.data[53], vb_15_elem.data[54], vb_15_elem.data[55], vb_15_elem.data[56], vb_15_elem.data[57], vb_15_elem.data[58], vb_15_elem.data[59]);
        fade = unpackFloat32x2_(vb_15_elem.data[60], vb_15_elem.data[61], vb_15_elem.data[62], vb_15_elem.data[63], vb_15_elem.data[64], vb_15_elem.data[65], vb_15_elem.data[66], vb_15_elem.data[67]);
        taper_depth = unpackFloat32x4_(vb_15_elem.data[68], vb_15_elem.data[69], vb_15_elem.data[70], vb_15_elem.data[71], vb_15_elem.data[72], vb_15_elem.data[73], vb_15_elem.data[74], vb_15_elem.data[75], vb_15_elem.data[76], vb_15_elem.data[77], vb_15_elem.data[78], vb_15_elem.data[79], vb_15_elem.data[80], vb_15_elem.data[81], vb_15_elem.data[82], vb_15_elem.data[83]);
        taper = unpackFloat32x4_(vb_15_elem.data[84], vb_15_elem.data[85], vb_15_elem.data[86], vb_15_elem.data[87], vb_15_elem.data[88], vb_15_elem.data[89], vb_15_elem.data[90], vb_15_elem.data[91], vb_15_elem.data[92], vb_15_elem.data[93], vb_15_elem.data[94], vb_15_elem.data[95], vb_15_elem.data[96], vb_15_elem.data[97], vb_15_elem.data[98], vb_15_elem.data[99]);
    }
    metal::float2 local = {};
    VertexOut out = {};
    metal::float2 corner = metal::float2(((vertex_ & 1u) == 1u) ? 1.0 : -1.0, ((vertex_ & 2u) == 2u) ? 1.0 : -1.0);
    float _e30 = locals.shadow.w;
    float _e33 = locals.feather;
    float reach = _e30 + (0.5 * _e33);
    float _e39 = locals.feather;
    float margin = reach + (0.5 * _e39);
    metal::float2 extent = metal::float2((half_extent.x + (metal::abs(shear) * half_extent.y)) + margin, half_extent.y + margin);
    local = corner * extent;
    metal::float2 span = span_ramp.xy;
    local.y = (corner.y > 0.0) ? metal::min(extent.y, span.y) : metal::max(-(extent.y), span.x);
    metal::float2 _e69 = locals.pitch_dir;
    float _e71 = local.x;
    metal::float2 _e76 = locals.depth_dir;
    float _e78 = local.y;
    metal::float2 pos = (center + (_e69 * _e71)) + (_e76 * _e78);
    metal::float2 _e83 = locals.origin_points;
    metal::float2 in_viewport = pos - _e83;
    float _e93 = locals.viewport_points.x;
    float _e103 = locals.viewport_points.y;
    out.position = metal::float4(((2.0 * in_viewport.x) / _e93) - 1.0, 1.0 - ((2.0 * in_viewport.y) / _e103), 0.0, 1.0);
    metal::float2 _e111 = local;
    out.local = _e111;
    out.half_extent = half_extent;
    out.shear = shear;
    float _e118 = locals.shadow.w;
    out.outline_reach = _e118;
    out.lead = lead;
    out.taper_depth = taper_depth;
    out.taper = taper;
    out.core = core;
    out.outline = outline;
    out.at = pos;
    out.who = who;
    float _e129 = locals.feather;
    out.feather = _e129;
    out.ramp = span_ramp.zw;
    out.fade = fade;
    VertexOut _e133 = out;
    const auto _tmp = _e133;
    return vs_noteOutput { _tmp.position, _tmp.local, _tmp.half_extent, _tmp.shear, _tmp.outline_reach, _tmp.lead, _tmp.taper_depth, _tmp.taper, _tmp.core, _tmp.outline, _tmp.at, _tmp.who, _tmp.feather, _tmp.ramp, _tmp.fade };
}
