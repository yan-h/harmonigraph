// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint buffer_size15;
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
struct PadOut {
    metal::float4 position;
    float pad;
    char _pad2[12];
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

struct vs_distance_padOutput {
    metal::float4 position [[position]];
    float pad [[user(loc0), flat]];
};
struct vb_15_type { metal::uchar data[64]; };
vertex vs_distance_padOutput vs_distance_pad(
  uint vertex_ [[vertex_id]]
, constant Locals& locals [[buffer(0)]]
, uint i_id [[instance_id]]
, const device vb_15_type* vb_15_in [[buffer(15)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(1)]]
) {
    metal::float4 box_rect = {};
    metal::float4 box_cell = {};
    metal::float4 box_meta = {};
    metal::float4 box_who = {};
    if (i_id < (_buffer_sizes.buffer_size15 / 64)) {
        const vb_15_type vb_15_elem = vb_15_in[i_id];
        box_rect = unpackFloat32x4_(vb_15_elem.data[0], vb_15_elem.data[1], vb_15_elem.data[2], vb_15_elem.data[3], vb_15_elem.data[4], vb_15_elem.data[5], vb_15_elem.data[6], vb_15_elem.data[7], vb_15_elem.data[8], vb_15_elem.data[9], vb_15_elem.data[10], vb_15_elem.data[11], vb_15_elem.data[12], vb_15_elem.data[13], vb_15_elem.data[14], vb_15_elem.data[15]);
        box_cell = unpackFloat32x4_(vb_15_elem.data[16], vb_15_elem.data[17], vb_15_elem.data[18], vb_15_elem.data[19], vb_15_elem.data[20], vb_15_elem.data[21], vb_15_elem.data[22], vb_15_elem.data[23], vb_15_elem.data[24], vb_15_elem.data[25], vb_15_elem.data[26], vb_15_elem.data[27], vb_15_elem.data[28], vb_15_elem.data[29], vb_15_elem.data[30], vb_15_elem.data[31]);
        box_meta = unpackFloat32x4_(vb_15_elem.data[32], vb_15_elem.data[33], vb_15_elem.data[34], vb_15_elem.data[35], vb_15_elem.data[36], vb_15_elem.data[37], vb_15_elem.data[38], vb_15_elem.data[39], vb_15_elem.data[40], vb_15_elem.data[41], vb_15_elem.data[42], vb_15_elem.data[43], vb_15_elem.data[44], vb_15_elem.data[45], vb_15_elem.data[46], vb_15_elem.data[47]);
        box_who = unpackFloat32x4_(vb_15_elem.data[48], vb_15_elem.data[49], vb_15_elem.data[50], vb_15_elem.data[51], vb_15_elem.data[52], vb_15_elem.data[53], vb_15_elem.data[54], vb_15_elem.data[55], vb_15_elem.data[56], vb_15_elem.data[57], vb_15_elem.data[58], vb_15_elem.data[59], vb_15_elem.data[60], vb_15_elem.data[61], vb_15_elem.data[62], vb_15_elem.data[63]);
    }
    PadOut out = {};
    bool local = {};
    metal::float2 corner = metal::float2(((vertex_ & 1u) == 1u) ? 1.0 : 0.0, ((vertex_ & 2u) == 2u) ? 1.0 : 0.0);
    metal::float2 texel_1 = box_cell.xy + (corner * box_cell.zw);
    metal::float4 _e26 = no_quad();
    metal::float2 _e29 = locals.shadow_atlas_size;
    metal::float4 _e31 = cell_clip(texel_1, _e29, 1.0);
    if (box_who.y >= 0.5) {
        bool _e37 = cell_packed(box_cell);
        local = _e37;
    } else {
        local = false;
    }
    bool _e39 = local;
    out.position = _e39 ? _e31 : _e26;
    out.pad = box_who.z;
    PadOut _e43 = out;
    const auto _tmp = _e43;
    return vs_distance_padOutput { _tmp.position, _tmp.pad };
}
