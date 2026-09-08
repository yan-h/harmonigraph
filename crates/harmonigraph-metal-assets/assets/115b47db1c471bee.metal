// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint buffer_size15;
    uint buffer_size14;
};

struct DotShadowLocals {
    metal::float2 screen_points;
    metal::float2 shadow_atlas_size;
    metal::float4 shadow;
    metal::float4 falloff;
};
struct DotShadowOut {
    metal::float4 position;
    metal::float2 local;
    float radius;
    char _pad3[4];
    metal::float2 at;
    uint who;
    char _pad5[4];
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
metal::float4 unpackFloat32x4_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7, uint b8, uint b9, uint b10, uint b11, uint b12, uint b13, uint b14, uint b15) {
    return metal::float4(as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0), as_type<float>(b7 << 24 | b6 << 16 | b5 << 8 | b4), as_type<float>(b11 << 24 | b10 << 16 | b9 << 8 | b8), as_type<float>(b15 << 24 | b14 << 16 | b13 << 8 | b12));
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

metal::float2 dot_corner(
    uint vertex_1
) {
    return metal::float2(((vertex_1 & 1u) == 1u) ? 1.0 : -1.0, ((vertex_1 & 2u) == 2u) ? 1.0 : -1.0);
}
uint naga_f2u32(float value) {
    return static_cast<uint>(metal::clamp(value, 0.0, 4294967000.0));
}


struct vs_dot_shadow_cellOutput {
    metal::float4 position [[position]];
    metal::float2 local [[user(loc0), center_perspective]];
    float radius [[user(loc1), flat]];
    metal::float2 at [[user(loc2), center_perspective]];
    uint who [[user(loc3), flat]];
};
struct vb_15_type { metal::uchar data[16]; };
struct vb_14_type { metal::uchar data[64]; };
vertex vs_dot_shadow_cellOutput vs_dot_shadow_cell(
  uint vertex_ [[vertex_id]]
, constant DotShadowLocals& dot_locals [[buffer(0)]]
, uint i_id [[instance_id]]
, const device vb_15_type* vb_15_in [[buffer(15)]]
, const device vb_14_type* vb_14_in [[buffer(14)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(1)]]
) {
    metal::float2 center = {};
    float radius = {};
    metal::float4 color = {};
    if (i_id < (_buffer_sizes.buffer_size15 / 16)) {
        const vb_15_type vb_15_elem = vb_15_in[i_id];
        center = unpackFloat32x2_(vb_15_elem.data[0], vb_15_elem.data[1], vb_15_elem.data[2], vb_15_elem.data[3], vb_15_elem.data[4], vb_15_elem.data[5], vb_15_elem.data[6], vb_15_elem.data[7]);
        radius = unpackFloat32_(vb_15_elem.data[8], vb_15_elem.data[9], vb_15_elem.data[10], vb_15_elem.data[11]);
        color = unpackUnorm8x4_(vb_15_elem.data[12], vb_15_elem.data[13], vb_15_elem.data[14], vb_15_elem.data[15]);
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
    DotShadowOut out = {};
    bool local = {};
    metal::float2 _e8 = dot_corner(vertex_);
    metal::float2 corner = 0.5 * (_e8 + metal::float2(1.0));
    metal::float2 point = box_rect.xy + (corner * box_rect.zw);
    metal::float2 _e19 = cell_texel(point, box_rect, box_cell, box_meta.x);
    metal::float4 _e22 = no_quad();
    metal::float2 _e25 = dot_locals.shadow_atlas_size;
    metal::float4 _e27 = cell_clip(_e19, _e25, 1.0);
    bool _e28 = cell_packed(box_cell);
    if (_e28) {
        local = box_who.y < 0.5;
    } else {
        local = false;
    }
    bool _e35 = local;
    out.position = _e35 ? _e27 : _e22;
    out.local = point - center;
    out.radius = radius;
    out.at = point;
    out.who = naga_f2u32(box_who.x + 0.5);
    DotShadowOut _e46 = out;
    const auto _tmp = _e46;
    return vs_dot_shadow_cellOutput { _tmp.position, _tmp.local, _tmp.radius, _tmp.at, _tmp.who };
}
