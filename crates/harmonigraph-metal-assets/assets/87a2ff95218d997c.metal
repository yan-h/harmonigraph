// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct Vertex {
    metal::float4 position;
    metal::float2 uv;
    char _pad2[8];
};

struct vs_fullscreenInput {
};
struct vs_fullscreenOutput {
    metal::float4 position [[position]];
    metal::float2 uv [[user(loc0), center_perspective]];
};
vertex vs_fullscreenOutput vs_fullscreen(
  uint vertex_ [[vertex_id]]
) {
    Vertex out = {};
    metal::float2 uv = metal::float2(static_cast<float>((vertex_ << 1u) & 2u), static_cast<float>(vertex_ & 2u));
    out.position = metal::float4((uv * metal::float2(2.0, -2.0)) + metal::float2(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    Vertex _e24 = out;
    const auto _tmp = _e24;
    return vs_fullscreenOutput { _tmp.position, _tmp.uv };
}
