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
    metal::float2 ramp;
    char _pad15[8];
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
metal::float2 unpackFloat32x2_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7) {
    return metal::float2(as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0), as_type<float>(b7 << 24 | b6 << 16 | b5 << 8 | b4));
}
float unpackFloat32_(uint b0, uint b1, uint b2, uint b3) {
    return as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0);
}
metal::float4 unpackUnorm8x4_(metal::uchar b0, metal::uchar b1, metal::uchar b2, metal::uchar b3) {
    return metal::float4(float(b0) / 255.0f, float(b1) / 255.0f, float(b2) / 255.0f, float(b3) / 255.0f);
}
metal::float4 unpackFloat32x4_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7, uint b8, uint b9, uint b10, uint b11, uint b12, uint b13, uint b14, uint b15) {
    return metal::float4(as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0), as_type<float>(b7 << 24 | b6 << 16 | b5 << 8 | b4), as_type<float>(b11 << 24 | b10 << 16 | b9 << 8 | b8), as_type<float>(b15 << 24 | b14 << 16 | b13 << 8 | b12));
}

struct vs_noteOutput {
    metal::float4 position [[position]];
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
    metal::float2 ramp [[user(loc13), flat]];
    metal::float4 reads [[user(loc14), flat]];
};
struct vb_15_type { metal::uchar data[80]; };
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
    float outline_reach = {};
    float lead = {};
    float lead_fade = {};
    float lead_alpha = {};
    float cap_reach = {};
    metal::float4 core = {};
    metal::float4 outline = {};
    metal::float4 span_ramp = {};
    metal::float4 reads = {};
    if (who < (_buffer_sizes.buffer_size15 / 80)) {
        const vb_15_type vb_15_elem = vb_15_in[who];
        center = unpackFloat32x2_(vb_15_elem.data[0], vb_15_elem.data[1], vb_15_elem.data[2], vb_15_elem.data[3], vb_15_elem.data[4], vb_15_elem.data[5], vb_15_elem.data[6], vb_15_elem.data[7]);
        half_extent = unpackFloat32x2_(vb_15_elem.data[8], vb_15_elem.data[9], vb_15_elem.data[10], vb_15_elem.data[11], vb_15_elem.data[12], vb_15_elem.data[13], vb_15_elem.data[14], vb_15_elem.data[15]);
        shear = unpackFloat32_(vb_15_elem.data[16], vb_15_elem.data[17], vb_15_elem.data[18], vb_15_elem.data[19]);
        outline_reach = unpackFloat32_(vb_15_elem.data[20], vb_15_elem.data[21], vb_15_elem.data[22], vb_15_elem.data[23]);
        lead = unpackFloat32_(vb_15_elem.data[24], vb_15_elem.data[25], vb_15_elem.data[26], vb_15_elem.data[27]);
        lead_fade = unpackFloat32_(vb_15_elem.data[28], vb_15_elem.data[29], vb_15_elem.data[30], vb_15_elem.data[31]);
        lead_alpha = unpackFloat32_(vb_15_elem.data[32], vb_15_elem.data[33], vb_15_elem.data[34], vb_15_elem.data[35]);
        cap_reach = unpackFloat32_(vb_15_elem.data[36], vb_15_elem.data[37], vb_15_elem.data[38], vb_15_elem.data[39]);
        core = unpackUnorm8x4_(vb_15_elem.data[40], vb_15_elem.data[41], vb_15_elem.data[42], vb_15_elem.data[43]);
        outline = unpackUnorm8x4_(vb_15_elem.data[44], vb_15_elem.data[45], vb_15_elem.data[46], vb_15_elem.data[47]);
        span_ramp = unpackFloat32x4_(vb_15_elem.data[48], vb_15_elem.data[49], vb_15_elem.data[50], vb_15_elem.data[51], vb_15_elem.data[52], vb_15_elem.data[53], vb_15_elem.data[54], vb_15_elem.data[55], vb_15_elem.data[56], vb_15_elem.data[57], vb_15_elem.data[58], vb_15_elem.data[59], vb_15_elem.data[60], vb_15_elem.data[61], vb_15_elem.data[62], vb_15_elem.data[63]);
        reads = unpackFloat32x4_(vb_15_elem.data[64], vb_15_elem.data[65], vb_15_elem.data[66], vb_15_elem.data[67], vb_15_elem.data[68], vb_15_elem.data[69], vb_15_elem.data[70], vb_15_elem.data[71], vb_15_elem.data[72], vb_15_elem.data[73], vb_15_elem.data[74], vb_15_elem.data[75], vb_15_elem.data[76], vb_15_elem.data[77], vb_15_elem.data[78], vb_15_elem.data[79]);
    }
    metal::float2 local = {};
    VertexOut out = {};
    metal::float2 corner = metal::float2(((vertex_ & 1u) == 1u) ? 1.0 : -1.0, ((vertex_ & 2u) == 2u) ? 1.0 : -1.0);
    float _e32 = locals.shadow.w;
    float _e35 = locals.feather;
    float reach = _e32 + (0.5 * _e35);
    float _e41 = locals.feather;
    float margin = reach + (0.5 * _e41);
    metal::float2 extent = metal::float2((half_extent.x + (metal::abs(shear) * half_extent.y)) + margin, half_extent.y + margin);
    local = corner * extent;
    metal::float2 span = span_ramp.xy;
    local.y = (corner.y > 0.0) ? metal::min(extent.y, span.y) : metal::max(-(extent.y), span.x);
    metal::float2 _e71 = locals.pitch_dir;
    float _e73 = local.x;
    metal::float2 _e78 = locals.depth_dir;
    float _e80 = local.y;
    metal::float2 pos = (center + (_e71 * _e73)) + (_e78 * _e80);
    metal::float2 _e85 = locals.origin_points;
    metal::float2 in_viewport = pos - _e85;
    float _e95 = locals.viewport_points.x;
    float _e105 = locals.viewport_points.y;
    out.position = metal::float4(((2.0 * in_viewport.x) / _e95) - 1.0, 1.0 - ((2.0 * in_viewport.y) / _e105), 0.0, 1.0);
    metal::float2 _e113 = local;
    out.local = _e113;
    out.half_extent = half_extent;
    out.shear = shear;
    float _e120 = locals.shadow.w;
    out.outline_reach = _e120;
    out.lead = lead;
    out.lead_fade = lead_fade;
    out.lead_alpha = lead_alpha;
    out.cap_reach = cap_reach;
    out.core = core;
    out.outline = outline;
    out.at = pos;
    out.who = who;
    float _e132 = locals.feather;
    out.feather = _e132;
    out.ramp = span_ramp.zw;
    out.reads = reads;
    VertexOut _e136 = out;
    const auto _tmp = _e136;
    return vs_noteOutput { _tmp.position, _tmp.local, _tmp.half_extent, _tmp.shear, _tmp.outline_reach, _tmp.lead, _tmp.lead_fade, _tmp.lead_alpha, _tmp.cap_reach, _tmp.core, _tmp.outline, _tmp.at, _tmp.who, _tmp.feather, _tmp.ramp, _tmp.reads };
}
