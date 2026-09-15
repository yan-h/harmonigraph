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

float wispy_light(
    metal::float2 p_1,
    metal::float2 drift
) {
    float _e3 = nebula_noise(p_1 + drift);
    float _e9 = nebula_noise((p_1 + metal::float2(8.3, 2.7)) - drift);
    metal::float2 bend = metal::float2(_e3, _e9) - metal::float2(0.5);
    metal::float2 folded = p_1 + (bend * 0.6);
    metal::float2 q = (folded * metal::float2(0.9, 2.8)) - drift;
    float _e22 = nebula_noise(q);
    float _e31 = nebula_noise((q * 2.07) + metal::float2(3.1, 7.4));
    float _e41 = nebula_noise((q * 4.13) + metal::float2(6.7, 1.2));
    float filament = ((_e22 * 0.56) + (_e31 * 0.29)) + (_e41 * 0.15);
    return metal::smoothstep(0.27, 0.73, filament);
}

float puffy_noise(
    metal::float2 p_2
) {
    float _e1 = nebula_noise(p_2);
    float _e10 = nebula_noise((p_2 * 2.03) + metal::float2(5.2, 1.7));
    float _e20 = nebula_noise((p_2 * 4.13) + metal::float2(1.3, 9.1));
    float _e30 = nebula_noise((p_2 * 8.17) + metal::float2(7.8, 3.2));
    return (((_e1 * 0.5) + (_e10 * 0.27)) + (_e20 * 0.15)) + (_e30 * 0.08);
}

float puffy_light(
    metal::float2 p_3,
    metal::float2 drift_1,
    float swell
) {
    float _e8 = nebula_noise((p_3 * 0.37) + (drift_1 * 0.45));
    float _e12 = nebula_noise((p_3 * 0.65) + drift_1);
    float _e20 = nebula_noise(((p_3 * 0.65) + metal::float2(8.3, 2.7)) - drift_1);
    metal::float2 shoulder = metal::float2(_e12, _e20) - metal::float2(0.5);
    metal::float2 inflated = ((p_3 * (1.0 - (swell * 0.22))) + (shoulder * 0.9)) + drift_1;
    float _e39 = puffy_noise(inflated * (0.65 + (_e8 * 0.25)));
    return metal::smoothstep(0.28, 0.7, (_e8 * 0.2) + (_e39 * 0.8));
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

float cloud_thickness(
    metal::float2 p_4,
    float swell_1,
    constant Cloud& cloud
) {
    uint _e4 = cloud.style;
    if (_e4 == 4u) {
        metal::float4 _e11 = cloud.motion;
        float _e15 = puffy_light(p_4 * 1.6, _e11.xy * 0.6, swell_1);
        return _e15;
    }
    metal::float4 _e23 = cloud.motion;
    float _e25 = wispy_light(p_4 * (1.0 - (swell_1 * 0.08)), _e23.xy);
    return _e25;
}

metal::float4 illuminated_color(
    float level_1,
    metal::float2 position_1,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    metal::float4 _e2 = density_color(level_1, lut, cloud);
    float strength_1 = cloud.motion.w;
    if (strength_1 <= 0.0) {
        return _e2;
    }
    float _e11 = cloud.ppp;
    metal::float2 point = position_1 / metal::float2(_e11);
    metal::float2 _e16 = cloud.origin;
    metal::float2 _e20 = cloud.size;
    metal::float2 uv_1 = (point - _e16) / _e20;
    metal::float4 _e25 = wide_light.sample(cloud_sampler, uv_1, metal::level(0.0));
    float incident = _e25.x;
    float presence = metal::smoothstep(0.0, 0.5, strength_1);
    if (incident <= 0.00001) {
        return metal::float4(_e2.xyz * (1.0 - presence), 1.0);
    }
    float _e41 = cloud.field.w;
    float _e47 = cloud.motion.z;
    float cell_size = (metal::max(_e41, 1.0) * _e47) / 5.0;
    metal::float4 _e53 = cloud.field;
    metal::float4 _e58 = cloud.field;
    metal::float2 p_5 = ((point - _e53.xy) - (_e58.zw * 0.5)) / metal::float2(cell_size);
    float phase = (p_5.x * 0.43) + (p_5.y * 0.37);
    float _e75 = cloud.breath.x;
    float _e85 = cloud.breath.y;
    float wave = (0.5 + (metal::sin(_e75 + phase) / 3.0)) + (metal::sin(_e85 + (phase * 1.7)) / 6.0);
    float _e96 = cloud.breath.z;
    float swell_2 = _e96 * (wave - 0.5);
    float _e100 = cloud_thickness(p_5, swell_2, cloud);
    metal::float2 _e106 = cloud.size;
    metal::float2 reach = metal::float2(cell_size * 0.4) / _e106;
    metal::float4 _e115 = wide_light.sample(cloud_sampler, uv_1 - metal::float2(reach.x, 0.0), metal::level(0.0));
    float left = _e115.x;
    metal::float4 _e124 = wide_light.sample(cloud_sampler, uv_1 + metal::float2(reach.x, 0.0), metal::level(0.0));
    float right = _e124.x;
    metal::float4 _e133 = wide_light.sample(cloud_sampler, uv_1 - metal::float2(0.0, reach.y), metal::level(0.0));
    float above = _e133.x;
    metal::float4 _e142 = wide_light.sample(cloud_sampler, uv_1 + metal::float2(0.0, reach.y), metal::level(0.0));
    float below = _e142.x;
    metal::float2 gradient = metal::float2(right - left, below - above);
    metal::float2 toward_light = (gradient / metal::float2(metal::max(metal::length(gradient), 0.00001))) * 0.25;
    float _e155 = cloud_thickness(p_5 + toward_light, swell_2, cloud);
    float _e159 = cloud_thickness(p_5 + (toward_light * 2.7), swell_2, cloud);
    float face = metal::clamp(0.4 + ((_e100 - _e155) * 2.6), 0.08, 1.3);
    float shelter = 1.0 - (metal::max(_e159 - _e100, 0.0) * 0.25);
    float density = metal::mix(1.0, 0.3 + (0.7 * _e100), strength_1);
    float lighting = density * (1.05 + ((0.65 * face) * shelter));
    metal::float4 _e189 = density_color(incident * 1.45, lut, cloud);
    metal::float3 light = _e189.xyz;
    metal::float3 body = (light * lighting) * (1.15 + (swell_2 * 0.12));
    return metal::float4(metal::mix(_e2.xyz, body, presence), 1.0);
}

metal::float4 material_color(
    float level_2,
    metal::float2 position_2,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    bool local = {};
    uint _e4 = cloud.style;
    if (!((_e4 == 3u))) {
        uint _e12 = cloud.style;
        local = _e12 == 4u;
    } else {
        local = true;
    }
    bool _e16 = local;
    if (_e16) {
        metal::float4 _e17 = illuminated_color(level_2, position_2, lut, wide_light, cloud_sampler, cloud);
        return _e17;
    }
    metal::float4 _e18 = density_color(level_2, lut, cloud);
    return _e18;
}

metal::float4 backdrop_color(
    metal::float2 position_3,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    float _e2 = baked_density(position_3, close_light, cloud_sampler, cloud);
    float _e3 = smoothed_level(0.0, _e2, cloud);
    metal::float4 _e4 = material_color(_e3, position_3, lut, wide_light, cloud_sampler, cloud);
    return _e4;
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
, metal::float4 position_4 [[position]]
, metal::texture2d<float, metal::access::sample> lut [[texture(0)]]
, metal::texture2d<float, metal::access::sample> close_light [[texture(1)]]
, metal::texture2d<float, metal::access::sample> wide_light [[texture(2)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(2)]]
) {
    const VertexOut in = { position_4, varyings.slab, varyings.t };
    metal::float4 _e3 = backdrop_color(in.position.xy, lut, close_light, wide_light, cloud_sampler, cloud);
    return fs_cloud_backdrop_gammaOutput { _e3 };
}
