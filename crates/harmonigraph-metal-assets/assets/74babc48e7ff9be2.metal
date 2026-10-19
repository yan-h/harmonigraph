// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint buffer_size15;
    uint buffer_size14;
};

struct Locals {
    metal::float2 screen_points;
    metal::float2 atlas_size;
    metal::float2 mark_atlas_size;
    metal::float2 filter_axis;
    float pixels_per_point;
    float shadow_depth;
    metal::float2 shadow_atlas_size;
    float node_occlusion;
    float _pad0_;
    metal::float2 _pad1_;
};
struct VertexOut {
    metal::float4 position;
    metal::float2 texel;
    metal::float2 uv_min;
    metal::float2 uv_max;
    char _pad4[8];
    metal::float4 fill;
    uint sheet;
    char _pad6[4];
    metal::float2 sheet_size;
    metal::float2 points;
    float who;
    char _pad9[4];
};
constant uint OCCLUDER_HEADER = 5u;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;
constant float GLOW_BASE = 0.8;
constant uint SHEET_MARK = 1u;
constant float PATCH_MARGIN = 0.5;
constant float FILTER_TAP = 0.25;
constant float CELL_FOOTPRINT_TAPS = 8.0;
metal::float4 unpackFloat32x4_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7, uint b8, uint b9, uint b10, uint b11, uint b12, uint b13, uint b14, uint b15) {
    return metal::float4(as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0), as_type<float>(b7 << 24 | b6 << 16 | b5 << 8 | b4), as_type<float>(b11 << 24 | b10 << 16 | b9 << 8 | b8), as_type<float>(b15 << 24 | b14 << 16 | b13 << 8 | b12));
}
metal::float4 unpackUnorm8x4_(metal::uchar b0, metal::uchar b1, metal::uchar b2, metal::uchar b3) {
    return metal::float4(float(b0) / 255.0f, float(b1) / 255.0f, float(b2) / 255.0f, float(b3) / 255.0f);
}
uint unpackUint32_(uint b0, uint b1, uint b2, uint b3) {
    return (b3 << 24 | b2 << 16 | b1 << 8 | b0);
}

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

metal::float2 cell_texel(
    metal::float2 points,
    metal::float4 rect_1,
    metal::float4 cell_1,
    float k
) {
    return cell_1.xy + ((points - rect_1.xy) * k);
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

VertexOut glyph_vertex(
    uint vertex_1,
    metal::float4 rect_2,
    metal::float4 uv_1,
    metal::float4 fill_1,
    uint sheet_1,
    float grow,
    constant Locals& locals
) {
    VertexOut out_1 = {};
    metal::float2 corner = metal::float2(((vertex_1 & 1u) == 1u) ? 1.0 : 0.0, ((vertex_1 & 2u) == 2u) ? 1.0 : 0.0);
    float _e24 = locals.pixels_per_point;
    float reach = (0.75 / _e24) + grow;
    metal::float2 pos = (rect_2.xy - metal::float2(reach)) + (corner * (rect_2.zw + metal::float2(2.0 * reach)));
    out_1.position = metal::float4(pos, 0.0, 1.0);
    float _e44 = locals.pixels_per_point;
    float texel_reach = reach * _e44;
    out_1.texel = (uv_1.xy - metal::float2(texel_reach)) + (corner * ((uv_1.zw - uv_1.xy) + metal::float2(2.0 * texel_reach)));
    out_1.uv_min = uv_1.xy;
    out_1.uv_max = uv_1.zw;
    metal::float4 _e65 = out_1.position;
    out_1.points = _e65.xy;
    out_1.who = 0.0;
    out_1.fill = fill_1;
    out_1.sheet = sheet_1;
    metal::float2 _e74 = locals.atlas_size;
    metal::float2 _e77 = locals.mark_atlas_size;
    out_1.sheet_size = (sheet_1 == SHEET_MARK) ? _e77 : _e74;
    VertexOut _e81 = out_1;
    return _e81;
}

struct vs_glyph_cellOutput {
    metal::float4 position [[position]];
    metal::float2 texel [[user(loc0), center_perspective]];
    metal::float2 uv_min [[user(loc1), flat]];
    metal::float2 uv_max [[user(loc2), flat]];
    metal::float4 fill [[user(loc3), flat]];
    uint sheet [[user(loc5), flat]];
    metal::float2 sheet_size [[user(loc6), flat]];
    metal::float2 points [[user(loc7), center_perspective]];
    float who [[user(loc8), flat]];
};
struct vb_15_type { metal::uchar data[44]; };
struct vb_14_type { metal::uchar data[64]; };
vertex vs_glyph_cellOutput vs_glyph_cell(
  uint vertex_ [[vertex_id]]
, constant Locals& locals [[buffer(0)]]
, uint i_id [[instance_id]]
, const device vb_15_type* vb_15_in [[buffer(15)]]
, const device vb_14_type* vb_14_in [[buffer(14)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(1)]]
) {
    metal::float4 rect = {};
    metal::float4 uv = {};
    metal::float4 fill = {};
    metal::float4 rim = {};
    uint sheet = {};
    if (i_id < (_buffer_sizes.buffer_size15 / 44)) {
        const vb_15_type vb_15_elem = vb_15_in[i_id];
        rect = unpackFloat32x4_(vb_15_elem.data[0], vb_15_elem.data[1], vb_15_elem.data[2], vb_15_elem.data[3], vb_15_elem.data[4], vb_15_elem.data[5], vb_15_elem.data[6], vb_15_elem.data[7], vb_15_elem.data[8], vb_15_elem.data[9], vb_15_elem.data[10], vb_15_elem.data[11], vb_15_elem.data[12], vb_15_elem.data[13], vb_15_elem.data[14], vb_15_elem.data[15]);
        uv = unpackFloat32x4_(vb_15_elem.data[16], vb_15_elem.data[17], vb_15_elem.data[18], vb_15_elem.data[19], vb_15_elem.data[20], vb_15_elem.data[21], vb_15_elem.data[22], vb_15_elem.data[23], vb_15_elem.data[24], vb_15_elem.data[25], vb_15_elem.data[26], vb_15_elem.data[27], vb_15_elem.data[28], vb_15_elem.data[29], vb_15_elem.data[30], vb_15_elem.data[31]);
        fill = unpackUnorm8x4_(vb_15_elem.data[32], vb_15_elem.data[33], vb_15_elem.data[34], vb_15_elem.data[35]);
        rim = unpackUnorm8x4_(vb_15_elem.data[36], vb_15_elem.data[37], vb_15_elem.data[38], vb_15_elem.data[39]);
        sheet = unpackUint32_(vb_15_elem.data[40], vb_15_elem.data[41], vb_15_elem.data[42], vb_15_elem.data[43]);
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
    VertexOut _e15 = glyph_vertex(vertex_, rect, uv, fill, sheet, 1.0 / metal::max(box_meta.x, 0.000001), locals);
    out = _e15;
    metal::float4 _e18 = out.position;
    metal::float2 _e21 = cell_texel(_e18.xy, box_rect, box_cell, box_meta.x);
    metal::float4 _e23 = no_quad();
    metal::float2 _e26 = locals.shadow_atlas_size;
    metal::float4 _e28 = cell_clip(_e21, _e26, 1.0);
    bool _e29 = cell_packed(box_cell);
    out.position = _e29 ? _e28 : _e23;
    VertexOut _e31 = out;
    const auto _tmp = _e31;
    return vs_glyph_cellOutput { _tmp.position, _tmp.texel, _tmp.uv_min, _tmp.uv_max, _tmp.fill, _tmp.sheet, _tmp.sheet_size, _tmp.points, _tmp.who };
}
