// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint buffer_size15;
};

struct VertexOutput {
    metal::float2 tex_coord;
    char _pad1[8];
    metal::float4 color;
    metal::float4 position;
};
struct Locals {
    metal::float2 screen_size;
    uint dithering;
    uint predictable_texture_filtering;
};
metal::float2 unpackFloat32x2_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7) {
    return metal::float2(as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0), as_type<float>(b7 << 24 | b6 << 16 | b5 << 8 | b4));
}
uint unpackUint32_(uint b0, uint b1, uint b2, uint b3) {
    return (b3 << 24 | b2 << 16 | b1 << 8 | b0);
}

metal::float4 unpack_color(
    uint color
) {
    return metal::float4(static_cast<float>(color & 255u), static_cast<float>((color >> 8u) & 255u), static_cast<float>((color >> 16u) & 255u), static_cast<float>((color >> 24u) & 255u)) / metal::float4(255.0);
}

metal::float4 position_from_screen(
    metal::float2 screen_pos,
    constant Locals& r_locals
) {
    float _e7 = r_locals.screen_size.x;
    float _e17 = r_locals.screen_size.y;
    return metal::float4(((2.0 * screen_pos.x) / _e7) - 1.0, 1.0 - ((2.0 * screen_pos.y) / _e17), 0.0, 1.0);
}

struct vs_mainOutput {
    metal::float2 tex_coord [[user(loc0), center_perspective]];
    metal::float4 color [[user(loc1), center_perspective]];
    metal::float4 position [[position]];
};
struct vb_15_type { metal::uchar data[20]; };
vertex vs_mainOutput vs_main(
  constant Locals& r_locals [[buffer(0)]]
, uint v_id [[vertex_id]]
, const device vb_15_type* vb_15_in [[buffer(15)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(1)]]
) {
    metal::float2 a_pos = {};
    metal::float2 a_tex_coord = {};
    uint a_color = {};
    if (v_id < (_buffer_sizes.buffer_size15 / 20)) {
        const vb_15_type vb_15_elem = vb_15_in[v_id];
        a_pos = unpackFloat32x2_(vb_15_elem.data[0], vb_15_elem.data[1], vb_15_elem.data[2], vb_15_elem.data[3], vb_15_elem.data[4], vb_15_elem.data[5], vb_15_elem.data[6], vb_15_elem.data[7]);
        a_tex_coord = unpackFloat32x2_(vb_15_elem.data[8], vb_15_elem.data[9], vb_15_elem.data[10], vb_15_elem.data[11], vb_15_elem.data[12], vb_15_elem.data[13], vb_15_elem.data[14], vb_15_elem.data[15]);
        a_color = unpackUint32_(vb_15_elem.data[16], vb_15_elem.data[17], vb_15_elem.data[18], vb_15_elem.data[19]);
    }
    VertexOutput out = {};
    out.tex_coord = a_tex_coord;
    metal::float4 _e6 = unpack_color(a_color);
    out.color = _e6;
    metal::float4 _e8 = position_from_screen(a_pos, r_locals);
    out.position = _e8;
    VertexOutput _e9 = out;
    const auto _tmp = _e9;
    return vs_mainOutput { _tmp.tex_coord, _tmp.color, _tmp.position };
}
