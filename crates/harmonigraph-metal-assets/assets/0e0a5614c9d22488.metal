// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct Locals {
    metal::float2 origin_points;
    metal::float2 viewport_points;
    float min_midi;
    float span;
    float spectrum_min_midi;
    float bins_per_semitone;
    float level0_;
    float level_per_step;
    float level_per_midi;
    uint rows;
    uint bins;
    uint stride;
    uint capacity;
    uint first_slot;
    uint run_slabs;
    uint detail;
    float stroke_sigma;
    float prominence_steps;
    uint peak_stride;
    float gather_sigma_slabs;
    uint peak_header;
    uint peak_bands;
    float peak_band;
    uint _pad0_;
    uint _pad1_;
    uint _pad2_;
};
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
    float ppp;
    float cloud;
    float _pad0_;
    float _pad1_;
    float _pad2_;
};

metal::float3 linear_from_gamma_rgb(
    metal::float3 srgb
) {
    metal::bool3 cutoff = srgb < metal::float3(0.04045);
    metal::float3 lower = srgb / metal::float3(12.92);
    metal::float3 higher = metal::pow((srgb + metal::float3(0.055)) / metal::float3(1.055), metal::float3(2.4));
    return metal::select(higher, lower, cutoff);
}

float baked_density(
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
    return _e17.x;
}

float diffused_level(
    float core,
    float material,
    constant Cloud& cloud
) {
    float _e4 = cloud.diffusion;
    float _e9 = cloud.diffusion;
    float raw = (1.0 - _e4) * (1.0 - _e9);
    return metal::mix(material, core, raw);
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

float backdrop_level(
    VertexOut in_1,
    constant Locals& locals,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    float _e3 = baked_density(in_1.position.xy, close_light, cloud_sampler, cloud);
    uint _e6 = locals.detail;
    if (_e6 == 1u) {
        float _e11 = cloud.cloud;
        return _e11 * _e3;
    }
    float _e14 = diffused_level(0.0, _e3, cloud);
    return _e14;
}

struct fs_cloud_backdrop_linearInput {
    float slab [[user(loc0), center_perspective]];
    float t [[user(loc1), center_perspective]];
};
struct fs_cloud_backdrop_linearOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_cloud_backdrop_linearOutput fs_cloud_backdrop_linear(
  fs_cloud_backdrop_linearInput varyings [[stage_in]]
, metal::float4 position_1 [[position]]
, constant Locals& locals [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> lut [[texture(0)]]
, metal::texture2d<float, metal::access::sample> close_light [[texture(1)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(3)]]
) {
    const VertexOut in = { position_1, varyings.slab, varyings.t };
    float _e1 = backdrop_level(in, locals, close_light, cloud_sampler, cloud);
    metal::float4 _e2 = density_color(_e1, lut);
    metal::float3 _e4 = linear_from_gamma_rgb(_e2.xyz);
    return fs_cloud_backdrop_linearOutput { metal::float4(_e4, 1.0) };
}
