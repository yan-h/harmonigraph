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
metal::float2 unpackFloat32x2_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7) {
    return metal::float2(as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0), as_type<float>(b7 << 24 | b6 << 16 | b5 << 8 | b4));
}
float unpackFloat32_(uint b0, uint b1, uint b2, uint b3) {
    return as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0);
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
    float lead [[user(loc4), flat]];
    float lead_fade [[user(loc5), flat]];
    float lead_alpha [[user(loc6), flat]];
    float cap_reach [[user(loc7), flat]];
    metal::float4 core [[user(loc8), flat]];
    metal::float4 outline [[user(loc9), flat]];
    metal::float2 at [[user(loc10), center_perspective]];
    uint who [[user(loc11), flat]];
};
struct vb_15_type { metal::uchar data[48]; };
vertex vs_noteOutput vs_note(
  uint vertex_ [[vertex_id]]
, uint who [[instance_id]]
, constant Locals& locals [[buffer(0)]]
, const device vb_15_type* vb_15_in [[buffer(15)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(2)]]
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
    if (who < (_buffer_sizes.buffer_size15 / 48)) {
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
    }
    VertexOut out = {};
    metal::float2 corner = metal::float2(((vertex_ & 1u) == 1u) ? 1.0 : -1.0, ((vertex_ & 2u) == 2u) ? 1.0 : -1.0);
    float skew = metal::sqrt(1.0 + (shear * shear));
    float _e34 = locals.shadow.w;
    float _e37 = locals.feather;
    float reach = _e34 + (0.5 * _e37);
    float _e45 = locals.feather;
    float half_depth = (half_extent.y + reach) + (0.5 * _e45);
    metal::float2 extent = metal::float2((half_extent.x + (metal::abs(shear) * half_depth)) + (skew * reach), half_depth);
    metal::float2 local = corner * extent;
    metal::float2 _e59 = locals.pitch_dir;
    metal::float2 _e65 = locals.depth_dir;
    metal::float2 pos = (center + (_e59 * local.x)) + (_e65 * local.y);
    metal::float2 _e71 = locals.origin_points;
    metal::float2 in_viewport = pos - _e71;
    float _e81 = locals.viewport_points.x;
    float _e91 = locals.viewport_points.y;
    out.position = metal::float4(((2.0 * in_viewport.x) / _e81) - 1.0, 1.0 - ((2.0 * in_viewport.y) / _e91), 0.0, 1.0);
    out.local = local;
    out.half_extent = half_extent;
    out.shear = shear;
    float _e105 = locals.shadow.w;
    out.outline_reach = _e105;
    out.lead = lead;
    out.lead_fade = lead_fade;
    out.lead_alpha = lead_alpha;
    out.cap_reach = cap_reach;
    out.core = core;
    out.outline = outline;
    out.at = pos;
    out.who = who;
    VertexOut _e114 = out;
    const auto _tmp = _e114;
    return vs_noteOutput { _tmp.position, _tmp.local, _tmp.half_extent, _tmp.shear, _tmp.outline_reach, _tmp.lead, _tmp.lead_fade, _tmp.lead_alpha, _tmp.cap_reach, _tmp.core, _tmp.outline, _tmp.at, _tmp.who };
}
