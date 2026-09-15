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
    float ppp;
    float spread;
    float contours;
    float contour_softness;
    uint style;
    uint _pad;
    metal::float4 motion;
    metal::float4 breath;
    metal::float4 field;
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

float smoothed_level(
    float core,
    float material,
    constant Cloud& cloud
) {
    metal::float2 _e4 = cloud.step;
    if (metal::all(_e4 == metal::float2(0.0))) {
        return core;
    }
    return material;
}

float style_level(
    float level,
    constant Cloud& cloud
) {
    uint _e3 = cloud.style;
    if (_e3 != 2u) {
        return level;
    }
    float _e11 = cloud.contours;
    float x = metal::clamp(level, 0.0, 1.0) * _e11;
    float _e15 = cloud.contour_softness;
    float _e16 = metal::fwidth(x);
    float edge = metal::min(0.5, metal::max(_e15, _e16 * 0.5));
    float _e32 = cloud.contours;
    float terraces = (metal::floor(x) + metal::smoothstep(0.5 - edge, 0.5 + edge, metal::fract(x))) / _e32;
    float _e39 = metal::fwidth(x);
    float strength = (0.9 * metal::smoothstep(0.0, 1.0, x)) * (1.0 - metal::smoothstep(0.5, 1.5, _e39));
    return metal::mix(level, terraces, strength);
}

float nebula_hash(
    metal::int2 cell
) {
    uint n = {};
    n = (as_type<uint>(cell.x) * 2654435769u) ^ as_type<uint>(cell.y);
    uint _e9 = n;
    uint _e10 = n;
    n = (_e9 ^ (_e10 >> 16u)) * 2146121005u;
    uint _e16 = n;
    uint _e17 = n;
    n = (_e16 ^ (_e17 >> 15u)) * 2221713035u;
    uint _e23 = n;
    uint _e24 = n;
    n = _e23 ^ (_e24 >> 16u);
    uint _e28 = n;
    return static_cast<float>(_e28 >> 8u) / 16777216.0;
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

float nebula_noise(
    metal::float2 p
) {
    metal::int2 cell_1 = naga_f2i32(metal::floor(p));
    metal::float2 f = metal::fract(p);
    metal::float2 w = (f * f) * (metal::float2(3.0) - (2.0 * f));
    float _e11 = nebula_hash(cell_1);
    float _e16 = nebula_hash(as_type<metal::int2>(as_type<metal::uint2>(cell_1) + as_type<metal::uint2>(metal::int2(1, 0))));
    float _e23 = nebula_hash(as_type<metal::int2>(as_type<metal::uint2>(cell_1) + as_type<metal::uint2>(metal::int2(0, 1))));
    float _e28 = nebula_hash(as_type<metal::int2>(as_type<metal::uint2>(cell_1) + as_type<metal::uint2>(metal::int2(1, 1))));
    return metal::mix(metal::mix(_e11, _e16, w.x), metal::mix(_e23, _e28, w.x), w.y);
}

float cloud_sample(
    metal::float2 point,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    bool local = {};
    bool local_1 = {};
    bool local_2 = {};
    metal::float2 _e3 = cloud.origin;
    metal::float2 _e7 = cloud.size;
    metal::float2 uv_1 = (point - _e3) / _e7;
    if (!(metal::any(uv_1 < metal::float2(0.0)))) {
        local = metal::any(uv_1 > metal::float2(1.0));
    } else {
        local = true;
    }
    bool _e21 = local;
    if (!(_e21)) {
        metal::float4 _e27 = cloud.field;
        local_1 = metal::any(point < _e27.xy);
    } else {
        local_1 = true;
    }
    bool _e32 = local_1;
    if (!(_e32)) {
        metal::float4 _e38 = cloud.field;
        metal::float4 _e42 = cloud.field;
        local_2 = metal::any(point > (_e38.xy + _e42.zw));
    } else {
        local_2 = true;
    }
    bool _e48 = local_2;
    if (_e48) {
        return 0.0;
    }
    metal::float4 _e53 = close_light.sample(cloud_sampler, uv_1, metal::level(0.0));
    return _e53.x;
}

float cloud_density(
    metal::float2 position_1,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    float _e3 = cloud.ppp;
    metal::float2 point_1 = position_1 / metal::float2(_e3);
    float _e9 = cloud.field.w;
    float _e15 = cloud.motion.z;
    float cell_size = (metal::max(_e9, 1.0) * _e15) / 5.0;
    metal::float4 _e21 = cloud.field;
    metal::float4 _e26 = cloud.field;
    metal::float2 p_2 = ((point_1 - _e21.xy) - (_e26.zw * 0.5)) / metal::float2(cell_size);
    metal::float4 _e35 = cloud.motion;
    metal::float2 drift = _e35.xy;
    float _e38 = nebula_noise(p_2 + drift);
    float _e44 = nebula_noise((p_2 + metal::float2(8.3, 2.7)) - drift);
    metal::float2 bend = metal::float2(_e38, _e44) - metal::float2(0.5);
    metal::float2 folded = p_2 + (bend * 2.4);
    float _e59 = nebula_noise(((folded * 2.3) - drift) + metal::float2(3.1, 7.4));
    float _e67 = nebula_noise(((folded * 2.3) + drift) + metal::float2(6.7, 1.2));
    metal::float2 curl = metal::float2(_e59, _e67) - metal::float2(0.5);
    float phase = (p_2.x * 0.8) + (p_2.y * 0.5);
    float _e82 = cloud.breath.x;
    float _e92 = cloud.breath.y;
    float wave = (0.5 + (metal::sin(_e82 + phase) / 3.0)) + (metal::sin(_e92 + (phase * 1.7)) / 6.0);
    float _e103 = cloud.breath.z;
    float breath = 1.0 - (_e103 * (1.0 - wave));
    float _e112 = cloud.motion.w;
    float reach = (cell_size * _e112) * (0.65 + (0.35 * breath));
    metal::float2 carried = point_1 + (((bend * 1.5) + (curl * 0.65)) * reach);
    metal::float2 strand = (metal::float2(curl.y, -(curl.x)) * reach) * 0.35;
    float _e133 = cloud_sample(carried, close_light, cloud_sampler, cloud);
    float _e135 = cloud_sample(carried + strand, close_light, cloud_sampler, cloud);
    float _e137 = cloud_sample(carried - strand, close_light, cloud_sampler, cloud);
    float filament = (_e135 + _e137) * 0.5;
    float _e142 = nebula_noise(folded + drift);
    float density = 0.22 + (0.96 * metal::smoothstep(0.2, 0.78, (_e142 * 0.7) + ((curl.x + 0.5) * 0.3)));
    float _e161 = cloud.motion.w;
    float _e168 = cloud.motion.w;
    return (metal::mix(_e133, filament, 0.35 * _e161) * metal::mix(1.0, density, _e168)) * breath;
}

float puffy_noise(
    metal::float2 p_1
) {
    float _e1 = nebula_noise(p_1);
    float _e10 = nebula_noise((p_1 * 2.03) + metal::float2(5.2, 1.7));
    float _e20 = nebula_noise((p_1 * 4.13) + metal::float2(1.3, 9.1));
    return ((_e1 * 0.55) + (_e10 * 0.3)) + (_e20 * 0.15);
}

float puffy_density(
    metal::float2 position_2,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    float _e3 = cloud.ppp;
    metal::float2 point_2 = position_2 / metal::float2(_e3);
    float _e9 = cloud.field.w;
    float _e15 = cloud.motion.z;
    float cell_size_1 = (metal::max(_e9, 1.0) * _e15) / 5.0;
    metal::float4 _e21 = cloud.field;
    metal::float4 _e26 = cloud.field;
    metal::float2 p_3 = ((point_2 - _e21.xy) - (_e26.zw * 0.5)) / metal::float2(cell_size_1);
    metal::float4 _e35 = cloud.motion;
    metal::float2 drift_1 = _e35.xy * 0.6;
    float _e44 = nebula_noise((p_3 * 0.37) + (drift_1 * 0.45));
    float _e48 = nebula_noise((p_3 * 0.65) + drift_1);
    float _e56 = nebula_noise(((p_3 * 0.65) + metal::float2(8.3, 2.7)) - drift_1);
    metal::float2 shoulder = metal::float2(_e48, _e56) - metal::float2(0.5);
    float phase_1 = ((p_3.x * 0.43) + (p_3.y * 0.37)) + _e44;
    float _e72 = cloud.breath.x;
    float _e82 = cloud.breath.y;
    float wave_1 = (0.5 + (metal::sin(_e72 + phase_1) / 3.0)) + (metal::sin(_e82 + (phase_1 * 1.7)) / 6.0);
    float _e93 = cloud.breath.z;
    float swell = _e93 * (wave_1 - 0.5);
    metal::float2 inflated = ((p_3 * (1.0 - (swell * 0.22))) + (shoulder * 0.9)) + drift_1;
    float _e111 = puffy_noise(inflated * (0.65 + (_e44 * 0.25)));
    float density_1 = 0.15 + (1.55 * metal::smoothstep(0.28, 0.69, (_e44 * 0.27) + (_e111 * 0.73)));
    float _e128 = cloud.motion.w;
    metal::float2 carried_1 = point_2 + (((shoulder * cell_size_1) * _e128) * (0.95 + (swell * 0.4)));
    float _e136 = cloud_sample(carried_1, close_light, cloud_sampler, cloud);
    float _e140 = cloud.motion.w;
    return (_e136 * metal::mix(1.0, density_1, _e140)) * (1.0 + (swell * 0.45));
}

uint naga_f2u32(float value) {
    return static_cast<uint>(metal::clamp(value, 0.0, 4294967000.0));
}

metal::float4 density_color(
    float raw_level,
    metal::texture2d<float, metal::access::sample> lut,
    constant Cloud& cloud
) {
    float _e1 = style_level(raw_level, cloud);
    uint levels = metal::uint2(lut.get_width(), lut.get_height()).x;
    float x_1 = (metal::clamp(_e1, 0.0, 1.0) * static_cast<float>(levels)) - 0.5;
    uint i = naga_f2u32(metal::clamp(metal::floor(x_1), 0.0, static_cast<float>(levels - 1u)));
    uint clamped_lod_e23 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e23 = lut.read(metal::min(metal::uint2(metal::uint2(i, 0u)), metal::uint2(lut.get_width(clamped_lod_e23), lut.get_height(clamped_lod_e23)) - 1), clamped_lod_e23);
    metal::float3 a = _e23.xyz;
    if (x_1 < 0.0) {
        return metal::float4((a * (x_1 + 0.5)) * 2.0, 1.0);
    }
    uint clamped_lod_e43 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e43 = lut.read(metal::min(metal::uint2(metal::uint2(metal::min(i + 1u, levels - 1u), 0u)), metal::uint2(lut.get_width(clamped_lod_e43), lut.get_height(clamped_lod_e43)) - 1), clamped_lod_e43);
    metal::float3 b = _e43.xyz;
    return metal::float4(metal::mix(a, b, metal::fract(x_1)), 1.0);
}

metal::float4 backdrop_color(
    metal::float2 position_3,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    uint _e3 = cloud.style;
    if (_e3 == 3u) {
        float _e6 = cloud_density(position_3, close_light, cloud_sampler, cloud);
        metal::float4 _e7 = density_color(_e6, lut, cloud);
        return _e7;
    }
    uint _e10 = cloud.style;
    if (_e10 == 4u) {
        float _e13 = puffy_density(position_3, close_light, cloud_sampler, cloud);
        metal::float4 _e14 = density_color(_e13, lut, cloud);
        return _e14;
    }
    float _e16 = baked_density(position_3, close_light, cloud_sampler, cloud);
    float _e17 = smoothed_level(0.0, _e16, cloud);
    metal::float4 _e18 = density_color(_e17, lut, cloud);
    return _e18;
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
, metal::float4 position_4 [[position]]
, metal::texture2d<float, metal::access::sample> lut [[texture(0)]]
, metal::texture2d<float, metal::access::sample> close_light [[texture(1)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(2)]]
) {
    const VertexOut in = { position_4, varyings.slab, varyings.t };
    metal::float4 _e3 = backdrop_color(in.position.xy, lut, close_light, cloud_sampler, cloud);
    metal::float3 _e5 = linear_from_gamma_rgb(_e3.xyz);
    return fs_cloud_backdrop_linearOutput { metal::float4(_e5, 1.0) };
}
