// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

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

metal::float4 disc_color(
    VertexOut in_1,
    constant Locals& locals
) {
    float _e4 = locals.feather.x;
    float f = metal::max(_e4, 0.000001);
    return in_1.color * metal::clamp(((in_1.radius - metal::length(in_1.local)) / f) + 0.5, 0.0, 1.0);
}

struct fs_disc_gammaInput {
    metal::float2 local [[user(loc0), center_perspective]];
    float radius [[user(loc1), flat]];
    metal::float4 color [[user(loc2), flat]];
};
struct fs_disc_gammaOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_disc_gammaOutput fs_disc_gamma(
  fs_disc_gammaInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Locals& locals [[buffer(0)]]
) {
    const VertexOut in = { position, varyings.local, varyings.radius, {}, varyings.color };
    metal::float4 _e1 = disc_color(in, locals);
    return fs_disc_gammaOutput { _e1 };
}
