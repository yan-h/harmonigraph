// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct VertexOut {
    metal::float4 position;
    float slab;
    float t;
    char _pad3[8];
};
struct Cloud {
    metal::float2 origin;
    metal::float2 size;
    metal::float2 step;
    float diffusion;
    float texture;
    float ppp;
    float _pad0_;
    float _pad1_;
    float _pad2_;
};

metal::float2 baked_density(
    metal::float2 position,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    float _e3 = cloud.ppp;
    metal::float2 _e8 = cloud.origin;
    metal::float2 _e12 = cloud.size;
    metal::float2 uv = ((position / metal::float2(_e3)) - _e8) / _e12;
    metal::float4 _e17 = close_light.sample(cloud_sampler, uv, metal::level(0.0));
    return _e17.xy;
}

float diffused_level(
    float core,
    metal::float2 material,
    constant Cloud& cloud
) {
    float retain = metal::smoothstep(0.45, 0.95, core);
    float _e8 = cloud.diffusion;
    float _e12 = cloud.diffusion;
    float level_1 = metal::mix(core, material.x, _e8) + ((_e12 * retain) * metal::max(core - material.x, 0.0));
    float _e22 = cloud.texture;
    float _e25 = cloud.diffusion;
    float texture = (_e22 * _e25) * (1.0 - metal::smoothstep(0.7, 1.0, level_1));
    return level_1 * metal::mix(1.0, material.y, texture);
}

uint naga_f2u32(float value) {
    return static_cast<uint>(metal::clamp(value, 0.0, 4294967000.0));
}

metal::float4 density_color(
    float level,
    metal::texture2d<float, metal::access::sample> lut
) {
    uint levels = metal::uint2(lut.get_width(), lut.get_height()).x;
    float x = (metal::clamp(level, 0.0, 1.0) * static_cast<float>(levels)) - 0.5;
    uint i = naga_f2u32(metal::clamp(metal::floor(x), 0.0, static_cast<float>(levels - 1u)));
    uint clamped_lod_e22 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e22 = lut.read(metal::min(metal::uint2(metal::uint2(i, 0u)), metal::uint2(lut.get_width(clamped_lod_e22), lut.get_height(clamped_lod_e22)) - 1), clamped_lod_e22);
    metal::float3 a = _e22.xyz;
    if (x < 0.0) {
        return metal::float4((a * (x + 0.5)) * 2.0, 1.0);
    }
    uint clamped_lod_e42 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e42 = lut.read(metal::min(metal::uint2(metal::uint2(metal::min(i + 1u, levels - 1u), 0u)), metal::uint2(lut.get_width(clamped_lod_e42), lut.get_height(clamped_lod_e42)) - 1), clamped_lod_e42);
    metal::float3 b = _e42.xyz;
    return metal::float4(metal::mix(a, b, metal::fract(x)), 1.0);
}

struct fs_cloud_backdrop_gammaInput {
    float slab [[user(loc0), center_perspective]];
    float t [[user(loc1), center_perspective]];
};
struct fs_cloud_backdrop_gammaOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_cloud_backdrop_gammaOutput fs_cloud_backdrop_gamma(
  fs_cloud_backdrop_gammaInput varyings [[stage_in]]
, metal::float4 position_1 [[position]]
, metal::texture2d<float, metal::access::sample> lut [[texture(0)]]
, metal::texture2d<float, metal::access::sample> close_light [[texture(1)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(2)]]
) {
    const VertexOut in = { position_1, varyings.slab, varyings.t };
    metal::float2 _e4 = baked_density(in.position.xy, close_light, cloud_sampler, cloud);
    float _e5 = diffused_level(0.0, _e4, cloud);
    metal::float4 _e6 = density_color(_e5, lut);
    return fs_cloud_backdrop_gammaOutput { _e6 };
}
