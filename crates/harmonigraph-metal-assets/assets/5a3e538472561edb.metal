// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint buffer_size15;
};

struct CellOut {
    metal::float4 position;
    metal::float4 bounds;
    float sigma;
    char _pad3[12];
};
constant float DISTANCE_KIND = 1.0;
constant float REACH = 3.0;
constant int MAX_RADIUS = 9;
constant float PEDESTAL = 0.011109;
metal::float4 unpackFloat32x4_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7, uint b8, uint b9, uint b10, uint b11, uint b12, uint b13, uint b14, uint b15) {
    return metal::float4(as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0), as_type<float>(b7 << 24 | b6 << 16 | b5 << 8 | b4), as_type<float>(b11 << 24 | b10 << 16 | b9 << 8 | b8), as_type<float>(b15 << 24 | b14 << 16 | b13 << 8 | b12));
}

metal::float4 no_quad(
) {
    return metal::float4(2.0, 2.0, 0.0, 1.0);
}

CellOut cell_quad(
    uint vertex_1,
    metal::float4 cell_1,
    metal::float4 cell_map_1,
    bool draws,
    metal::texture2d<float, metal::access::sample> src
) {
    CellOut out = {};
    metal::float2 corner = metal::float2(((vertex_1 & 1u) == 1u) ? 1.0 : 0.0, ((vertex_1 & 2u) == 2u) ? 1.0 : 0.0);
    metal::float2 atlas = static_cast<metal::float2>(metal::uint2(src.get_width(), src.get_height()));
    metal::float2 texel = cell_1.xy + (corner * cell_1.zw);
    out.position = metal::float4(((texel.x / atlas.x) * 2.0) - 1.0, 1.0 - ((texel.y / atlas.y) * 2.0), 0.0, 1.0);
    if (!(draws)) {
        metal::float4 _e47 = no_quad();
        out.position = _e47;
    }
    out.bounds = metal::float4(cell_1.xy, cell_1.xy + cell_1.zw);
    out.sigma = cell_map_1.y;
    CellOut _e56 = out;
    return _e56;
}

struct vs_cellOutput {
    metal::float4 position [[position]];
    metal::float4 bounds [[user(loc0), flat]];
    float sigma [[user(loc1), flat]];
};
struct vb_15_type { metal::uchar data[64]; };
vertex vs_cellOutput vs_cell(
  uint vertex_ [[vertex_id]]
, metal::texture2d<float, metal::access::sample> src [[texture(0)]]
, uint i_id [[instance_id]]
, const device vb_15_type* vb_15_in [[buffer(15)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(0)]]
) {
    metal::float4 rect = {};
    metal::float4 cell = {};
    metal::float4 cell_map = {};
    metal::float4 who = {};
    if (i_id < (_buffer_sizes.buffer_size15 / 64)) {
        const vb_15_type vb_15_elem = vb_15_in[i_id];
        rect = unpackFloat32x4_(vb_15_elem.data[0], vb_15_elem.data[1], vb_15_elem.data[2], vb_15_elem.data[3], vb_15_elem.data[4], vb_15_elem.data[5], vb_15_elem.data[6], vb_15_elem.data[7], vb_15_elem.data[8], vb_15_elem.data[9], vb_15_elem.data[10], vb_15_elem.data[11], vb_15_elem.data[12], vb_15_elem.data[13], vb_15_elem.data[14], vb_15_elem.data[15]);
        cell = unpackFloat32x4_(vb_15_elem.data[16], vb_15_elem.data[17], vb_15_elem.data[18], vb_15_elem.data[19], vb_15_elem.data[20], vb_15_elem.data[21], vb_15_elem.data[22], vb_15_elem.data[23], vb_15_elem.data[24], vb_15_elem.data[25], vb_15_elem.data[26], vb_15_elem.data[27], vb_15_elem.data[28], vb_15_elem.data[29], vb_15_elem.data[30], vb_15_elem.data[31]);
        cell_map = unpackFloat32x4_(vb_15_elem.data[32], vb_15_elem.data[33], vb_15_elem.data[34], vb_15_elem.data[35], vb_15_elem.data[36], vb_15_elem.data[37], vb_15_elem.data[38], vb_15_elem.data[39], vb_15_elem.data[40], vb_15_elem.data[41], vb_15_elem.data[42], vb_15_elem.data[43], vb_15_elem.data[44], vb_15_elem.data[45], vb_15_elem.data[46], vb_15_elem.data[47]);
        who = unpackFloat32x4_(vb_15_elem.data[48], vb_15_elem.data[49], vb_15_elem.data[50], vb_15_elem.data[51], vb_15_elem.data[52], vb_15_elem.data[53], vb_15_elem.data[54], vb_15_elem.data[55], vb_15_elem.data[56], vb_15_elem.data[57], vb_15_elem.data[58], vb_15_elem.data[59], vb_15_elem.data[60], vb_15_elem.data[61], vb_15_elem.data[62], vb_15_elem.data[63]);
    }
    CellOut _e8 = cell_quad(vertex_, cell, cell_map, who.y < 0.5, src);
    const auto _tmp = _e8;
    return vs_cellOutput { _tmp.position, _tmp.bounds, _tmp.sigma };
}
