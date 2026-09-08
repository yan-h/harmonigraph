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
    metal::float4 _pad;
};
struct SdfOut {
    metal::float4 position;
    metal::float2 near_texel;
    metal::float2 coarse_texel;
    metal::float4 near_bounds;
    metal::float4 coarse_bounds;
    metal::float2 scales;
    char _pad6[8];
};
constant float DISTANCE_KIND = 1.0;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_FALLOFF_FLOOR = 0.35;
constant float SHADOW_TAIL = 4.0;
constant float SHADOW_FALLOFF_FREE = 0.64025325;
constant float SHADOW_STOP = 2.0;
constant float SHADOW_INVISIBLE_FOLDS = 1.5586027;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;
constant uint SHEET_MARK = 1u;
constant float PATCH_MARGIN = 0.5;
constant float FILTER_TAP = 0.25;
constant float SDF_NEAR_PAD = 32.0;
constant float SDF_COARSE_PAD = 48.0;
constant float SDF_NEAR_BLEND = 8.0;
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
    bool local_6 = {};
    if (cell.z > 0.0) {
        local_6 = cell.w > 0.0;
    } else {
        local_6 = false;
    }
    bool _e10 = local_6;
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

struct vs_glyph_distance_cellOutput {
    metal::float4 position [[position]];
    metal::float2 near_texel [[user(loc0), center_perspective]];
    metal::float2 coarse_texel [[user(loc1), center_perspective]];
    metal::float4 near_bounds [[user(loc2), flat]];
    metal::float4 coarse_bounds [[user(loc3), flat]];
    metal::float2 scales [[user(loc4), flat]];
};
struct vb_15_type { metal::uchar data[92]; };
struct vb_14_type { metal::uchar data[64]; };
vertex vs_glyph_distance_cellOutput vs_glyph_distance_cell(
  uint vertex_ [[vertex_id]]
, constant Locals& locals [[buffer(0)]]
, uint i_id [[instance_id]]
, const device vb_15_type* vb_15_in [[buffer(15)]]
, const device vb_14_type* vb_14_in [[buffer(14)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(1)]]
) {
    metal::float4 rect = {};
    metal::float4 uv = {};
    metal::float4 sdf_rect = {};
    metal::float4 sdf_near = {};
    metal::float4 sdf_coarse = {};
    metal::float4 fill = {};
    metal::float4 rim = {};
    uint sheet = {};
    if (i_id < (_buffer_sizes.buffer_size15 / 92)) {
        const vb_15_type vb_15_elem = vb_15_in[i_id];
        rect = unpackFloat32x4_(vb_15_elem.data[0], vb_15_elem.data[1], vb_15_elem.data[2], vb_15_elem.data[3], vb_15_elem.data[4], vb_15_elem.data[5], vb_15_elem.data[6], vb_15_elem.data[7], vb_15_elem.data[8], vb_15_elem.data[9], vb_15_elem.data[10], vb_15_elem.data[11], vb_15_elem.data[12], vb_15_elem.data[13], vb_15_elem.data[14], vb_15_elem.data[15]);
        uv = unpackFloat32x4_(vb_15_elem.data[16], vb_15_elem.data[17], vb_15_elem.data[18], vb_15_elem.data[19], vb_15_elem.data[20], vb_15_elem.data[21], vb_15_elem.data[22], vb_15_elem.data[23], vb_15_elem.data[24], vb_15_elem.data[25], vb_15_elem.data[26], vb_15_elem.data[27], vb_15_elem.data[28], vb_15_elem.data[29], vb_15_elem.data[30], vb_15_elem.data[31]);
        sdf_rect = unpackFloat32x4_(vb_15_elem.data[32], vb_15_elem.data[33], vb_15_elem.data[34], vb_15_elem.data[35], vb_15_elem.data[36], vb_15_elem.data[37], vb_15_elem.data[38], vb_15_elem.data[39], vb_15_elem.data[40], vb_15_elem.data[41], vb_15_elem.data[42], vb_15_elem.data[43], vb_15_elem.data[44], vb_15_elem.data[45], vb_15_elem.data[46], vb_15_elem.data[47]);
        sdf_near = unpackFloat32x4_(vb_15_elem.data[48], vb_15_elem.data[49], vb_15_elem.data[50], vb_15_elem.data[51], vb_15_elem.data[52], vb_15_elem.data[53], vb_15_elem.data[54], vb_15_elem.data[55], vb_15_elem.data[56], vb_15_elem.data[57], vb_15_elem.data[58], vb_15_elem.data[59], vb_15_elem.data[60], vb_15_elem.data[61], vb_15_elem.data[62], vb_15_elem.data[63]);
        sdf_coarse = unpackFloat32x4_(vb_15_elem.data[64], vb_15_elem.data[65], vb_15_elem.data[66], vb_15_elem.data[67], vb_15_elem.data[68], vb_15_elem.data[69], vb_15_elem.data[70], vb_15_elem.data[71], vb_15_elem.data[72], vb_15_elem.data[73], vb_15_elem.data[74], vb_15_elem.data[75], vb_15_elem.data[76], vb_15_elem.data[77], vb_15_elem.data[78], vb_15_elem.data[79]);
        fill = unpackUnorm8x4_(vb_15_elem.data[80], vb_15_elem.data[81], vb_15_elem.data[82], vb_15_elem.data[83]);
        rim = unpackUnorm8x4_(vb_15_elem.data[84], vb_15_elem.data[85], vb_15_elem.data[86], vb_15_elem.data[87]);
        sheet = unpackUint32_(vb_15_elem.data[88], vb_15_elem.data[89], vb_15_elem.data[90], vb_15_elem.data[91]);
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
    bool local = {};
    bool local_1 = {};
    bool local_2 = {};
    bool local_3 = {};
    bool local_4 = {};
    SdfOut out = {};
    bool local_5 = {};
    metal::float2 corner = metal::float2(((vertex_ & 1u) == 1u) ? 1.0 : 0.0, ((vertex_ & 2u) == 2u) ? 1.0 : 0.0);
    metal::float2 point = box_rect.xy + (corner * box_rect.zw);
    metal::float2 near_span = sdf_near.zw - sdf_near.xy;
    metal::float2 coarse_span = sdf_coarse.zw - sdf_coarse.xy;
    if (near_span.x > 0.0) {
        local = near_span.y > 0.0;
    } else {
        local = false;
    }
    bool _e47 = local;
    if (_e47) {
        local_1 = coarse_span.x > 0.0;
    } else {
        local_1 = false;
    }
    bool _e54 = local_1;
    if (_e54) {
        local_2 = coarse_span.y > 0.0;
    } else {
        local_2 = false;
    }
    bool _e61 = local_2;
    if (_e61) {
        local_3 = sdf_rect.z > 0.0;
    } else {
        local_3 = false;
    }
    bool _e68 = local_3;
    if (_e68) {
        local_4 = sdf_rect.w > 0.0;
    } else {
        local_4 = false;
    }
    bool valid = local_4;
    metal::float2 _e78 = cell_texel(point, box_rect, box_cell, box_meta.x);
    metal::float4 _e80 = no_quad();
    metal::float2 _e83 = locals.shadow_atlas_size;
    metal::float4 _e85 = cell_clip(_e78, _e83, 1.0);
    if (valid) {
        bool _e88 = cell_packed(box_cell);
        local_5 = _e88;
    } else {
        local_5 = false;
    }
    bool _e90 = local_5;
    out.position = _e90 ? _e85 : _e80;
    metal::float2 relative = (point - sdf_rect.xy) / metal::max(sdf_rect.zw, metal::float2(0.000001));
    out.near_texel = sdf_near.xy + (relative * near_span);
    out.coarse_texel = sdf_coarse.xy + (relative * coarse_span);
    out.near_bounds = metal::float4(sdf_near.xy - metal::float2(32.0), sdf_near.zw + metal::float2(32.0));
    out.coarse_bounds = metal::float4(sdf_coarse.xy - metal::float2(48.0), sdf_coarse.zw + metal::float2(48.0));
    out.scales = metal::float2(0.5 * ((sdf_rect.z / metal::max(near_span.x, 0.000001)) + (sdf_rect.w / metal::max(near_span.y, 0.000001))), 0.5 * ((sdf_rect.z / metal::max(coarse_span.x, 0.000001)) + (sdf_rect.w / metal::max(coarse_span.y, 0.000001))));
    SdfOut _e155 = out;
    const auto _tmp = _e155;
    return vs_glyph_distance_cellOutput { _tmp.position, _tmp.near_texel, _tmp.coarse_texel, _tmp.near_bounds, _tmp.coarse_bounds, _tmp.scales };
}
