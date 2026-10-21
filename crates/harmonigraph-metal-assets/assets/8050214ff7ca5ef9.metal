// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct SurroundCut {
    metal::float4 cut;
};
struct CoverOut {
    metal::float4 position;
    metal::float2 uv;
    char _pad2[8];
};
struct Over {
    metal::packed_float3 color;
    float cut;
};

metal::float3 linear_from_gamma_rgb(
    metal::float3 srgb
) {
    metal::bool3 cutoff = srgb < metal::float3(0.04045);
    metal::float3 lower = srgb / metal::float3(12.92);
    metal::float3 higher = metal::pow((srgb + metal::float3(0.055)) / metal::float3(1.055), metal::float3(2.4));
    return metal::select(higher, lower, cutoff);
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

Over surround_over(
    CoverOut in_1,
    constant SurroundCut& surround,
    metal::texture2d<float, metal::access::sample> surround_texels
) {
    metal::int2 size = static_cast<metal::int2>(metal::uint2(surround_texels.get_width(), surround_texels.get_height()));
    metal::int2 texel = metal::min(naga_f2i32(in_1.uv * static_cast<metal::float2>(size)), as_type<metal::int2>(as_type<metal::uint2>(size) - as_type<metal::uint2>(metal::int2(1))));
    uint clamped_lod_e14 = metal::min(uint(0), surround_texels.get_num_mip_levels() - 1);
    metal::float4 strongest = surround_texels.read(metal::min(metal::uint2(texel), metal::uint2(surround_texels.get_width(clamped_lod_e14), surround_texels.get_height(clamped_lod_e14)) - 1), clamped_lod_e14);
    float _e18 = surround.cut.x;
    float cut = _e18 * strongest.w;
    return Over {strongest.xyz, cut};
}

struct fs_surround_linearInput {
    metal::float2 uv [[user(loc0), center_perspective]];
};
struct fs_surround_linearOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_surround_linearOutput fs_surround_linear(
  fs_surround_linearInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant SurroundCut& surround [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> surround_texels [[texture(0)]]
) {
    const CoverOut in = { position, varyings.uv };
    Over _e1 = surround_over(in, surround, surround_texels);
    metal::float3 _e3 = linear_from_gamma_rgb(_e1.color);
    return fs_surround_linearOutput { metal::float4(_e3 * _e1.cut, _e1.cut) };
}
