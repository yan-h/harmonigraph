// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint size1;
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
struct Dot {
    metal::float2 center;
    float radius;
    uint aura;
    uint first;
    uint candidates;
};
typedef Dot type_4[1];
struct DotOut {
    metal::float4 position;
    uint dot;
    char _pad2[12];
};

float cut_reach(
    constant DotLocals& dot_locals
) {
    float _e4 = dot_locals.surround.cut.y;
    float _e9 = dot_locals.surround.cut.x;
    return (_e9 > 0.0) ? _e4 : 0.0;
}

struct vs_dot_cutInput {
};
struct vs_dot_cutOutput {
    metal::float4 position [[position]];
    uint dot [[user(loc0), flat]];
};
vertex vs_dot_cutOutput vs_dot_cut(
  uint vertex_ [[vertex_id]]
, uint dot [[instance_id]]
, constant DotLocals& dot_locals [[buffer(0)]]
, device type_4 const& dots [[buffer(1)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    DotOut out = {};
    metal::float2 corner = metal::float2(((vertex_ & 1u) == 1u) ? 1.0 : -1.0, ((vertex_ & 2u) == 2u) ? 1.0 : -1.0);
    Dot at = dots[metal::min(unsigned(dot), (_buffer_sizes.size1 - 0 - 24) / 24)];
    float _e21 = cut_reach(dot_locals);
    float _e25 = dot_locals.feather;
    metal::float2 local = corner * ((at.radius + _e21) + _e25);
    metal::float2 _e32 = dot_locals.origin_points;
    metal::float2 in_viewport = (at.center + local) - _e32;
    float _e42 = dot_locals.viewport_points.x;
    float _e52 = dot_locals.viewport_points.y;
    out.position = metal::float4(((2.0 * in_viewport.x) / _e42) - 1.0, 1.0 - ((2.0 * in_viewport.y) / _e52), 0.0, 1.0);
    out.dot = dot;
    DotOut _e60 = out;
    const auto _tmp = _e60;
    return vs_dot_cutOutput { _tmp.position, _tmp.dot };
}
