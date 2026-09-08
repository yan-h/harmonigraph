// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint buffer_size15;
};

struct Locals {
    metal::float4 viewport;
    metal::float4 feather;
};
struct VertexOut {
    metal::float4 position;
    metal::float2 local;
    float radius;
    char _pad3[4];
    metal::float4 color;
};
metal::float2 unpackFloat32x2_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7) {
    return metal::float2(as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0), as_type<float>(b7 << 24 | b6 << 16 | b5 << 8 | b4));
}
float unpackFloat32_(uint b0, uint b1, uint b2, uint b3) {
    return as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0);
}
metal::float4 unpackUnorm8x4_(metal::uchar b0, metal::uchar b1, metal::uchar b2, metal::uchar b3) {
    return metal::float4(float(b0) / 255.0f, float(b1) / 255.0f, float(b2) / 255.0f, float(b3) / 255.0f);
}

struct vs_discOutput {
    metal::float4 position [[position]];
    metal::float2 local [[user(loc0), center_perspective]];
    float radius [[user(loc1), flat]];
    metal::float4 color [[user(loc2), flat]];
};
struct vb_15_type { metal::uchar data[16]; };
vertex vs_discOutput vs_disc(
  uint vertex_ [[vertex_id]]
, constant Locals& locals [[buffer(0)]]
, uint i_id [[instance_id]]
, const device vb_15_type* vb_15_in [[buffer(15)]]
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
    VertexOut out = {};
    metal::float2 corner = metal::float2(((vertex_ & 1u) == 1u) ? 1.0 : -1.0, ((vertex_ & 2u) == 2u) ? 1.0 : -1.0);
    float _e22 = locals.feather.x;
    metal::float2 local = corner * (radius + (0.5 * _e22));
    metal::float4 _e30 = locals.viewport;
    metal::float2 in_viewport = (center + local) - _e30.xy;
    float _e41 = locals.viewport.z;
    float _e51 = locals.viewport.w;
    out.position = metal::float4(((2.0 * in_viewport.x) / _e41) - 1.0, 1.0 - ((2.0 * in_viewport.y) / _e51), 0.0, 1.0);
    out.local = local;
    out.radius = radius;
    out.color = color;
    VertexOut _e61 = out;
    const auto _tmp = _e61;
    return vs_discOutput { _tmp.position, _tmp.local, _tmp.radius, _tmp.color };
}
