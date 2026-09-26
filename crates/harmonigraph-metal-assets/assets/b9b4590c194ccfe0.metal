// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint buffer_size15;
    uint buffer_size14;
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

bool cell_packed(
    metal::float4 cell
) {
    bool local_1 = {};
    if (cell.z > 0.0) {
        local_1 = cell.w > 0.0;
    } else {
        local_1 = false;
    }
    bool _e10 = local_1;
    return _e10;
}

metal::float2 cell_texel(
    metal::float2 points,
    metal::float4 rect,
    metal::float4 cell_1,
    float k
) {
    return cell_1.xy + ((points - rect.xy) * k);
}

metal::float4 no_quad(
) {
    return metal::float4(2.0, 2.0, 0.0, 1.0);
}

metal::float4 cell_clip(
    metal::float2 texel,
    metal::float2 size,
    float w
) {
    metal::float2 extent = metal::max(size, metal::float2(1.0));
    return metal::float4((((2.0 * texel.x) / extent.x) - 1.0) * w, (1.0 - ((2.0 * texel.y) / extent.y)) * w, 0.0, w);
}
uint naga_f2u32(float value) {
    return static_cast<uint>(metal::clamp(value, 0.0, 4294967000.0));
}


struct vs_shadow_cellOutput {
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
struct vb_15_type { metal::uchar data[104]; };
struct vb_14_type { metal::uchar data[64]; };
vertex vs_shadow_cellOutput vs_shadow_cell(
  uint vertex_ [[vertex_id]]
, constant Locals& locals [[buffer(0)]]
, uint i_id [[instance_id]]
, const device vb_15_type* vb_15_in [[buffer(15)]]
, const device vb_14_type* vb_14_in [[buffer(14)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(1)]]
) {
    metal::float2 center = {};
    metal::float2 half_extent = {};
    float shear = {};
    float outline_reach = {};
    metal::float4 lead = {};
    metal::float4 core = {};
    metal::float4 outline = {};
    metal::float4 taper_depth = {};
    metal::float4 taper = {};
    if (i_id < (_buffer_sizes.buffer_size15 / 104)) {
        const vb_15_type vb_15_elem = vb_15_in[i_id];
        center = unpackFloat32x2_(vb_15_elem.data[0], vb_15_elem.data[1], vb_15_elem.data[2], vb_15_elem.data[3], vb_15_elem.data[4], vb_15_elem.data[5], vb_15_elem.data[6], vb_15_elem.data[7]);
        half_extent = unpackFloat32x2_(vb_15_elem.data[8], vb_15_elem.data[9], vb_15_elem.data[10], vb_15_elem.data[11], vb_15_elem.data[12], vb_15_elem.data[13], vb_15_elem.data[14], vb_15_elem.data[15]);
        shear = unpackFloat32_(vb_15_elem.data[16], vb_15_elem.data[17], vb_15_elem.data[18], vb_15_elem.data[19]);
        outline_reach = unpackFloat32_(vb_15_elem.data[20], vb_15_elem.data[21], vb_15_elem.data[22], vb_15_elem.data[23]);
        lead = unpackFloat32x4_(vb_15_elem.data[24], vb_15_elem.data[25], vb_15_elem.data[26], vb_15_elem.data[27], vb_15_elem.data[28], vb_15_elem.data[29], vb_15_elem.data[30], vb_15_elem.data[31], vb_15_elem.data[32], vb_15_elem.data[33], vb_15_elem.data[34], vb_15_elem.data[35], vb_15_elem.data[36], vb_15_elem.data[37], vb_15_elem.data[38], vb_15_elem.data[39]);
        core = unpackUnorm8x4_(vb_15_elem.data[40], vb_15_elem.data[41], vb_15_elem.data[42], vb_15_elem.data[43]);
        outline = unpackUnorm8x4_(vb_15_elem.data[44], vb_15_elem.data[45], vb_15_elem.data[46], vb_15_elem.data[47]);
        taper_depth = unpackFloat32x4_(vb_15_elem.data[72], vb_15_elem.data[73], vb_15_elem.data[74], vb_15_elem.data[75], vb_15_elem.data[76], vb_15_elem.data[77], vb_15_elem.data[78], vb_15_elem.data[79], vb_15_elem.data[80], vb_15_elem.data[81], vb_15_elem.data[82], vb_15_elem.data[83], vb_15_elem.data[84], vb_15_elem.data[85], vb_15_elem.data[86], vb_15_elem.data[87]);
        taper = unpackFloat32x4_(vb_15_elem.data[88], vb_15_elem.data[89], vb_15_elem.data[90], vb_15_elem.data[91], vb_15_elem.data[92], vb_15_elem.data[93], vb_15_elem.data[94], vb_15_elem.data[95], vb_15_elem.data[96], vb_15_elem.data[97], vb_15_elem.data[98], vb_15_elem.data[99], vb_15_elem.data[100], vb_15_elem.data[101], vb_15_elem.data[102], vb_15_elem.data[103]);
    }
    metal::float4 box_rect = {};
    metal::float4 box_cell = {};
    metal::float4 box_meta = {};
    metal::float4 box_who = {};
    if (i_id < (_buffer_sizes.buffer_size14 / 64)) {
        const vb_14_type vb_14_elem = vb_14_in[i_id];
        box_rect = unpackFloat32x4_(vb_14_elem.data[0], vb_14_elem.data[1], vb_14_elem.data[2], vb_14_elem.data[3], vb_14_elem.data[4], vb_14_elem.data[5], vb_14_elem.data[6], vb_14_elem.data[7], vb_14_elem.data[8], vb_14_elem.data[9], vb_14_elem.data[10], vb_14_elem.data[11], vb_14_elem.data[12], vb_14_elem.data[13], vb_14_elem.data[14], vb_14_elem.data[15]);
        box_cell = unpackFloat32x4_(vb_14_elem.data[16], vb_14_elem.data[17], vb_14_elem.data[18], vb_14_elem.data[19], vb_14_elem.data[20], vb_14_elem.data[21], vb_14_elem.data[22], vb_14_elem.data[23], vb_14_elem.data[24], vb_14_elem.data[25], vb_14_elem.data[26], vb_14_elem.data[27], vb_14_elem.data[28], vb_14_elem.data[29], vb_14_elem.data[30], vb_14_elem.data[31]);
        box_meta = unpackFloat32x4_(vb_14_elem.data[32], vb_14_elem.data[33], vb_14_elem.data[34], vb_14_elem.data[35], vb_14_elem.data[36], vb_14_elem.data[37], vb_14_elem.data[38], vb_14_elem.data[39], vb_14_elem.data[40], vb_14_elem.data[41], vb_14_elem.data[42], vb_14_elem.data[43], vb_14_elem.data[44], vb_14_elem.data[45], vb_14_elem.data[46], vb_14_elem.data[47]);
        box_who = unpackFloat32x4_(vb_14_elem.data[48], vb_14_elem.data[49], vb_14_elem.data[50], vb_14_elem.data[51], vb_14_elem.data[52], vb_14_elem.data[53], vb_14_elem.data[54], vb_14_elem.data[55], vb_14_elem.data[56], vb_14_elem.data[57], vb_14_elem.data[58], vb_14_elem.data[59], vb_14_elem.data[60], vb_14_elem.data[61], vb_14_elem.data[62], vb_14_elem.data[63]);
    }
    VertexOut out = {};
    bool local = {};
    metal::float2 corner = metal::float2(((vertex_ & 1u) == 1u) ? 1.0 : 0.0, ((vertex_ & 2u) == 2u) ? 1.0 : 0.0);
    metal::float2 point = box_rect.xy + (corner * box_rect.zw);
    metal::float2 delta = point - center;
    metal::float2 _e36 = cell_texel(point, box_rect, box_cell, box_meta.x);
    metal::float4 _e38 = no_quad();
    metal::float2 _e41 = locals.shadow_atlas_size;
    metal::float4 _e43 = cell_clip(_e36, _e41, 1.0);
    bool _e44 = cell_packed(box_cell);
    if (_e44) {
        local = box_who.y < 0.5;
    } else {
        local = false;
    }
    bool _e51 = local;
    out.position = _e51 ? _e43 : _e38;
    metal::float2 _e56 = locals.pitch_dir;
    metal::float2 _e60 = locals.depth_dir;
    out.local = metal::float2(metal::dot(delta, _e56), metal::dot(delta, _e60));
    out.half_extent = half_extent;
    out.shear = shear;
    float _e69 = locals.shadow.w;
    out.outline_reach = _e69;
    out.lead = lead;
    out.taper_depth = taper_depth;
    out.taper = taper;
    out.core = core;
    out.outline = outline;
    out.at = point;
    out.who = naga_f2u32(box_who.x + 0.5);
    out.feather = 1.0 / metal::max(box_meta.x, 0.000001);
    out.ramp = metal::float2(0.0);
    out.fade = metal::float2(1.0);
    VertexOut _e93 = out;
    const auto _tmp = _e93;
    return vs_shadow_cellOutput { _tmp.position, _tmp.local, _tmp.half_extent, _tmp.shear, _tmp.outline_reach, _tmp.lead, _tmp.taper_depth, _tmp.taper, _tmp.core, _tmp.outline, _tmp.at, _tmp.who, _tmp.feather, _tmp.ramp, _tmp.fade };
}
