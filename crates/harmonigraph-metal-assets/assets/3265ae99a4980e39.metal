// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint buffer_size15;
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

metal::float2 dot_corner(
    uint vertex_1
) {
    return metal::float2(((vertex_1 & 1u) == 1u) ? 1.0 : -1.0, ((vertex_1 & 2u) == 2u) ? 1.0 : -1.0);
}

struct vs_dot_shadowOutput {
    metal::float4 position [[position]];
    metal::float2 local [[user(loc0), center_perspective]];
    float radius [[user(loc1), flat]];
    metal::float2 at [[user(loc2), center_perspective]];
    uint who [[user(loc3), flat]];
};
struct vb_15_type { metal::uchar data[16]; };
vertex vs_dot_shadowOutput vs_dot_shadow(
  uint vertex_ [[vertex_id]]
, uint who [[instance_id]]
, constant DotShadowLocals& dot_locals [[buffer(0)]]
, const device vb_15_type* vb_15_in [[buffer(15)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(2)]]
) {
    metal::float2 center = {};
    float radius = {};
    metal::float4 color = {};
    if (who < (_buffer_sizes.buffer_size15 / 16)) {
        const vb_15_type vb_15_elem = vb_15_in[who];
        center = unpackFloat32x2_(vb_15_elem.data[0], vb_15_elem.data[1], vb_15_elem.data[2], vb_15_elem.data[3], vb_15_elem.data[4], vb_15_elem.data[5], vb_15_elem.data[6], vb_15_elem.data[7]);
        radius = unpackFloat32_(vb_15_elem.data[8], vb_15_elem.data[9], vb_15_elem.data[10], vb_15_elem.data[11]);
        color = unpackUnorm8x4_(vb_15_elem.data[12], vb_15_elem.data[13], vb_15_elem.data[14], vb_15_elem.data[15]);
    }
    DotShadowOut out = {};
    metal::float2 _e5 = dot_corner(vertex_);
    float _e9 = dot_locals.shadow.w;
    metal::float2 local = _e5 * (radius + _e9);
    metal::float2 point = center + local;
    float _e21 = dot_locals.screen_points.x;
    float _e31 = dot_locals.screen_points.y;
    out.position = metal::float4(((2.0 * point.x) / _e21) - 1.0, 1.0 - ((2.0 * point.y) / _e31), 0.0, 1.0);
    out.local = local;
    out.radius = radius;
    out.at = point;
    out.who = who;
    DotShadowOut _e42 = out;
    const auto _tmp = _e42;
    return vs_dot_shadowOutput { _tmp.position, _tmp.local, _tmp.radius, _tmp.at, _tmp.who };
}
