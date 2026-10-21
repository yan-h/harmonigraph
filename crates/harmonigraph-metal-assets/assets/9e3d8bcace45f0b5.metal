// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint buffer_size15;
};

struct SurroundCut {
    metal::float4 cut;
};
struct DotLocals {
    SurroundCut surround;
    metal::float2 origin_points;
    metal::float2 viewport_points;
    float feather;
    float _feather_pad;
    metal::float2 _tail_pad;
};
struct DotOut {
    metal::float4 position;
    metal::float2 local;
    float radius;
    char _pad3[4];
    metal::float4 aura;
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

struct vs_dot_cutOutput {
    metal::float4 position [[position]];
    metal::float2 local [[user(loc0), center_perspective]];
    float radius [[user(loc1), flat]];
    metal::float4 aura [[user(loc2), flat]];
};
struct vb_15_type { metal::uchar data[16]; };
vertex vs_dot_cutOutput vs_dot_cut(
  uint vertex_ [[vertex_id]]
, constant DotLocals& dot_locals [[buffer(0)]]
, uint i_id [[instance_id]]
, const device vb_15_type* vb_15_in [[buffer(15)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(1)]]
) {
    metal::float2 center = {};
    float radius = {};
    metal::float4 aura = {};
    if (i_id < (_buffer_sizes.buffer_size15 / 16)) {
        const vb_15_type vb_15_elem = vb_15_in[i_id];
        center = unpackFloat32x2_(vb_15_elem.data[0], vb_15_elem.data[1], vb_15_elem.data[2], vb_15_elem.data[3], vb_15_elem.data[4], vb_15_elem.data[5], vb_15_elem.data[6], vb_15_elem.data[7]);
        radius = unpackFloat32_(vb_15_elem.data[8], vb_15_elem.data[9], vb_15_elem.data[10], vb_15_elem.data[11]);
        aura = unpackUnorm8x4_(vb_15_elem.data[12], vb_15_elem.data[13], vb_15_elem.data[14], vb_15_elem.data[15]);
    }
    DotOut out = {};
    metal::float2 corner = metal::float2(((vertex_ & 1u) == 1u) ? 1.0 : -1.0, ((vertex_ & 2u) == 2u) ? 1.0 : -1.0);
    float _e23 = dot_locals.surround.cut.y;
    float _e28 = dot_locals.surround.cut.x;
    float reach = (_e28 > 0.0) ? _e23 : 0.0;
    float _e36 = dot_locals.feather;
    metal::float2 local = corner * ((radius + reach) + _e36);
    metal::float2 _e42 = dot_locals.origin_points;
    metal::float2 in_viewport = (center + local) - _e42;
    float _e52 = dot_locals.viewport_points.x;
    float _e62 = dot_locals.viewport_points.y;
    out.position = metal::float4(((2.0 * in_viewport.x) / _e52) - 1.0, 1.0 - ((2.0 * in_viewport.y) / _e62), 0.0, 1.0);
    out.local = local;
    out.radius = radius;
    out.aura = aura;
    DotOut _e72 = out;
    const auto _tmp = _e72;
    return vs_dot_cutOutput { _tmp.position, _tmp.local, _tmp.radius, _tmp.aura };
}
