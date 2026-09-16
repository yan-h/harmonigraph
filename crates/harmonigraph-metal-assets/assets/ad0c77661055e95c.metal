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
    metal::float2 drift;
    float time;
    float cloud_depth;
    float cloud_scale;
    float cloud_cover;
    float cloud_billow;
    float cloud_fringe;
    float cloud_grain;
    float cloud_wash;
    float cloud_tide;
    float _pad2_;
};
struct Wash {
    float body;
    float tide;
};
struct type_12 {
    float inner[2];
};
constant metal::float2x2 CLOUD_TURN = metal::float2x2(metal::float2(0.8, 0.6), metal::float2(-0.6, 0.8));
constant float CLOUD_FLOOR = 0.05;

float density_decode(
    float value
) {
    float y = metal::max(value, 0.0);
    return (2.0 * y) / (0.1 + metal::sqrt(0.01 + (3.6 * y)));
}

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
    metal::float2 uv_3 = ((position / metal::float2(_e3)) - _e8) / _e12;
    metal::float4 _e17 = close_light.sample(cloud_sampler, uv_3, metal::level(0.0));
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

uint naga_f2u32(float value) {
    return static_cast<uint>(metal::clamp(value, 0.0, 4294967000.0));
}

metal::float3 palette_color(
    float level_1,
    metal::texture2d<float, metal::access::sample> lut
) {
    uint levels = metal::uint2(lut.get_width(), lut.get_height()).x;
    float x_1 = (metal::clamp(level_1, 0.0, 1.0) * static_cast<float>(levels)) - 0.5;
    uint i_2 = naga_f2u32(metal::clamp(metal::floor(x_1), 0.0, static_cast<float>(levels - 1u)));
    uint clamped_lod_e22 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e22 = lut.read(metal::min(metal::uint2(metal::uint2(i_2, 0u)), metal::uint2(lut.get_width(clamped_lod_e22), lut.get_height(clamped_lod_e22)) - 1), clamped_lod_e22);
    metal::float3 a = _e22.xyz;
    if (x_1 < 0.0) {
        return (a * (x_1 + 0.5)) * 2.0;
    }
    uint clamped_lod_e40 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e40 = lut.read(metal::min(metal::uint2(metal::uint2(metal::min(i_2 + 1u, levels - 1u), 0u)), metal::uint2(lut.get_width(clamped_lod_e40), lut.get_height(clamped_lod_e40)) - 1), clamped_lod_e40);
    metal::float3 b = _e40.xyz;
    return metal::mix(a, b, metal::fract(x_1));
}

metal::float4 density_color(
    float raw_level,
    metal::texture2d<float, metal::access::sample> lut,
    constant Cloud& cloud
) {
    float _e1 = style_level(raw_level, cloud);
    metal::float3 _e2 = palette_color(_e1, lut);
    return metal::float4(_e2, 1.0);
}

metal::float2 cloud_gradient(
    metal::int2 cell
) {
    uint n = {};
    n = (as_type<uint>(cell.x) * 2654435769u) ^ (as_type<uint>(cell.y) * 2246822507u);
    uint _e11 = n;
    uint _e12 = n;
    n = (_e11 ^ (_e12 >> 16u)) * 2146121005u;
    uint _e18 = n;
    uint _e19 = n;
    n = (_e18 ^ (_e19 >> 15u)) * 2221713035u;
    uint _e25 = n;
    uint _e26 = n;
    n = _e25 ^ (_e26 >> 16u);
    uint _e30 = n;
    float angle = static_cast<float>(_e30) * 0.0000000014629181;
    return metal::float2(metal::cos(angle), metal::sin(angle));
}

metal::float2 cloud_fade(
    metal::float2 t
) {
    return ((t * t) * t) * ((t * ((t * 6.0) - metal::float2(15.0))) + metal::float2(10.0));
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

float cloud_noise(
    metal::float2 p
) {
    metal::float2 base_1 = metal::floor(p);
    metal::float2 f = p - base_1;
    metal::int2 c = naga_f2i32(base_1);
    metal::float2 _e4 = cloud_gradient(c);
    metal::float2 _e9 = cloud_gradient(as_type<metal::int2>(as_type<metal::uint2>(c) + as_type<metal::uint2>(metal::int2(1, 0))));
    metal::float2 _e14 = cloud_gradient(as_type<metal::int2>(as_type<metal::uint2>(c) + as_type<metal::uint2>(metal::int2(0, 1))));
    metal::float2 _e19 = cloud_gradient(as_type<metal::int2>(as_type<metal::uint2>(c) + as_type<metal::uint2>(metal::int2(1, 1))));
    float n00_ = metal::dot(_e4, f);
    float n10_ = metal::dot(_e9, f - metal::float2(1.0, 0.0));
    float n01_ = metal::dot(_e14, f - metal::float2(0.0, 1.0));
    float n11_ = metal::dot(_e19, f - metal::float2(1.0, 1.0));
    metal::float2 _e36 = cloud_fade(f);
    return metal::mix(metal::mix(n00_, n10_, _e36.x), metal::mix(n01_, n11_, _e36.x), _e36.y) * 1.4;
}

float cloud_fbm(
    metal::float2 point,
    int octaves,
    float gain
) {
    metal::float2 p_1 = {};
    float amplitude = 1.0;
    float total = 0.0;
    float norm = 0.0;
    int i = 0;
    p_1 = point;
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            int _e30 = i;
            i = as_type<int>(as_type<uint>(_e30) + as_type<uint>(1));
        }
        loop_init = false;
        int _e12 = i;
        if (_e12 < octaves) {
        } else {
            break;
        }
        {
            float _e14 = total;
            float _e15 = amplitude;
            metal::float2 _e16 = p_1;
            float _e17 = cloud_noise(_e16);
            total = _e14 + (_e15 * _e17);
            float _e20 = norm;
            float _e21 = amplitude;
            norm = _e20 + _e21;
            metal::float2 _e24 = p_1;
            p_1 = (CLOUD_TURN * _e24) * 2.0;
            float _e28 = amplitude;
            amplitude = _e28 * gain;
        }
    }
    float _e33 = total;
    float _e34 = norm;
    return _e33 / metal::max(_e34, 0.0001);
}

float cloud_close(
    metal::float2 uv,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler
) {
    metal::float4 _e4 = close_light.sample(cloud_sampler, uv, metal::level(0.0));
    return _e4.x;
}

float cloud_wide(
    metal::float2 uv_1,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler
) {
    metal::float4 _e4 = wide_light.sample(cloud_sampler, uv_1, metal::level(0.0));
    float _e6 = density_decode(_e4.x);
    return _e6;
}

float cloud_surround(
    metal::float2 uv_2,
    metal::float2 reach,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler
) {
    metal::float2 d0_ = metal::float2(0.2887, 0.0);
    metal::float2 d1_ = metal::float2(-0.3687, 0.3377);
    metal::float2 d2_ = metal::float2(0.0564, -0.643);
    metal::float2 d3_ = metal::float2(0.4647, 0.6061);
    metal::float2 d4_ = metal::float2(-0.8528, -0.1508);
    metal::float2 d5_ = metal::float2(0.8078, -0.5139);
    float _e22 = cloud_wide(uv_2 + (d0_ * reach), wide_light, cloud_sampler);
    float _e25 = cloud_wide(uv_2 + (d1_ * reach), wide_light, cloud_sampler);
    float _e29 = cloud_wide(uv_2 + (d2_ * reach), wide_light, cloud_sampler);
    float _e33 = cloud_wide(uv_2 + (d3_ * reach), wide_light, cloud_sampler);
    float _e37 = cloud_wide(uv_2 + (d4_ * reach), wide_light, cloud_sampler);
    float _e41 = cloud_wide(uv_2 + (d5_ * reach), wide_light, cloud_sampler);
    return (((((_e22 + _e25) + _e29) + _e33) + _e37) + _e41) / 6.0;
}

float cloud_contrast(
    float centre,
    float around
) {
    return (centre - around) / metal::max(around, CLOUD_FLOOR);
}

Wash cloud_wash(
    float field,
    float threshold,
    float soft,
    float tide_width,
    float thickness,
    float fall,
    float pool
) {
    Wash out = {};
    float past = field - threshold;
    float a_1 = metal::smoothstep(-1.0, 1.0, past / metal::max(soft, 0.0001));
    out.body = a_1 * (1.0 - metal::exp(-(thickness) * metal::max(past, 0.0)));
    float edge_1 = metal::smoothstep(-1.0, 1.0, past / metal::max(tide_width, 0.0001));
    float weight = (1.0 - pool) + (pool * metal::clamp(0.5 + (0.5 * fall), 0.0, 1.0));
    out.tide = ((4.0 * edge_1) * (1.0 - edge_1)) * weight;
    float present = metal::smoothstep(0.0, 0.35 * metal::max(threshold, 0.0001), field);
    float _e55 = out.body;
    out.body = _e55 * present;
    float _e58 = out.tide;
    out.tide = _e58 * present;
    Wash _e60 = out;
    return _e60;
}

metal::float3 cloud_dilute(
    metal::float3 colour,
    float water
) {
    return metal::mix(colour, (colour * 0.3) + metal::float3(0.7), water);
}

metal::float3 scale_clouds(
    metal::float3 base,
    metal::float2 position_1,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    bool local = {};
    metal::float3 out_1 = {};
    int i_1 = 0;
    float _e4 = cloud.cloud_depth;
    if (!((_e4 <= 0.0))) {
        metal::float2 _e12 = cloud.step;
        local = metal::all(_e12 == metal::float2(0.0));
    } else {
        local = true;
    }
    bool _e18 = local;
    if (_e18) {
        return base;
    }
    float _e21 = cloud.ppp;
    metal::float2 _e26 = cloud.origin;
    metal::float2 pt = (position_1 / metal::float2(_e21)) - _e26;
    float _e30 = cloud.cloud_scale;
    float units = 5.0 / _e30;
    metal::float2 _e35 = cloud.size;
    float _e42 = cloud.size.y;
    metal::float2 _e48 = cloud.drift;
    metal::float2 q = (((pt - (_e35 * 0.5)) / metal::float2(_e42)) * units) + _e48;
    float _e53 = cloud.size.y;
    metal::float2 _e58 = cloud.size;
    metal::float2 unit = metal::float2(_e53 / units) / _e58;
    float _e62 = cloud.cloud_billow;
    float billow = 1.6 * _e62;
    float _e69 = cloud_fbm(q * 0.62, 3, 0.5);
    float _e78 = cloud_fbm((q * 0.62) + metal::float2(19.3, 7.1), 3, 0.5);
    metal::float2 warp = billow * metal::float2(_e69, _e78);
    metal::float2 _e83 = cloud.size;
    metal::float2 uv_4 = (pt / _e83) + (warp * unit);
    float _e89 = cloud.cloud_fringe;
    float _e100 = cloud_fbm((q * 2.6) + metal::float2(5.7, 12.9), 4, 0.55);
    float fringe = 1.0 + ((1.1 * _e89) * _e100);
    float _e104 = cloud_close(uv_4, close_light, cloud_sampler);
    float _e105 = cloud_wide(uv_4, wide_light, cloud_sampler);
    float _e108 = cloud_surround(uv_4, unit * 1.5, wide_light, cloud_sampler);
    float _e109 = cloud_contrast(_e105, _e108);
    float coarse = _e109 * fringe;
    float _e111 = cloud_contrast(_e104, _e105);
    float fine = _e111 * fringe;
    float _e119 = cloud_wide(uv_4 + metal::float2(0.0, unit.y * 0.35), wide_light, cloud_sampler);
    float fall_1 = metal::clamp((_e105 - _e119) * 8.0, -1.0, 1.0);
    float _e128 = cloud.cloud_cover;
    float sky = 0.62 - (0.52 * _e128);
    Wash _e137 = cloud_wash(coarse, sky, 0.05, 0.16, 3.4, fall_1, 0.75);
    Wash _e143 = cloud_wash(fine, sky * 1.25, 0.035, 0.11, 4.0, fall_1, 0.75);
    float _e147 = cloud.size.y;
    metal::float2 sheet = pt / metal::float2(_e147);
    float _e154 = cloud_fbm(sheet * 170.0, 2, 0.5);
    float _e165 = cloud_fbm((sheet * 48.0) + metal::float2(31.7, 3.3), 3, 0.5);
    float tooth = 0.5 + (0.5 * ((0.55 * _e154) + (0.45 * _e165)));
    float _e175 = cloud.cloud_grain;
    float _e180 = cloud.cloud_grain;
    float grain = (1.0 - _e175) + ((_e180 * 2.0) * tooth);
    float tint = metal::pow(metal::clamp(_e105 * 1.3, 0.0, 1.0), 0.7);
    metal::float3 _e199 = palette_color(metal::clamp((tint * 0.8) + 0.18, 0.0, 1.0), lut);
    out_1 = base;
    float depth = cloud.cloud_depth;
    float water_1 = cloud.cloud_wash;
    type_12 bodies = type_12 {{_e137.body * 0.88, _e143.body * 0.74}};
    type_12 tides = type_12 {{_e137.tide, _e143.tide}};
    type_12 waters = type_12 {{water_1, water_1 * 0.62}};
    uint2 loop_bound_1 = uint2(4294967295u);
    bool loop_init_1 = true;
    while(true) {
        if (metal::all(loop_bound_1 == uint2(0u))) { break; }
        loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
        if (!loop_init_1) {
            int _e250 = i_1;
            i_1 = as_type<int>(as_type<uint>(_e250) + as_type<uint>(1));
        }
        loop_init_1 = false;
        int _e222 = i_1;
        if (_e222 < 2) {
        } else {
            break;
        }
        {
            int _e225 = i_1;
            float a_2 = metal::clamp(bodies.inner[metal::min(unsigned(_e225), 1u)] * grain, 0.0, 1.0) * depth;
            metal::float3 _e232 = out_1;
            int _e233 = i_1;
            metal::float3 _e235 = cloud_dilute(_e199, waters.inner[metal::min(unsigned(_e233), 1u)]);
            out_1 = metal::mix(_e232, _e235, a_2);
            int _e237 = i_1;
            float _e241 = cloud.cloud_tide;
            float t_1 = metal::clamp((tides.inner[metal::min(unsigned(_e237), 1u)] * _e241) * grain, 0.0, 1.0) * depth;
            metal::float3 _e248 = out_1;
            out_1 = metal::mix(_e248, _e199, t_1);
        }
    }
    metal::float3 _e253 = out_1;
    return _e253;
}

metal::float4 clouded(
    float level_2,
    metal::float2 position_2,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    metal::float4 _e2 = density_color(level_2, lut, cloud);
    metal::float3 _e4 = scale_clouds(_e2.xyz, position_2, lut, close_light, wide_light, cloud_sampler, cloud);
    return metal::float4(_e4, 1.0);
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
, metal::float4 position_3 [[position]]
, metal::texture2d<float, metal::access::sample> lut [[texture(0)]]
, metal::texture2d<float, metal::access::sample> close_light [[texture(1)]]
, metal::texture2d<float, metal::access::sample> wide_light [[texture(2)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(2)]]
) {
    const VertexOut in = { position_3, varyings.slab, varyings.t };
    float _e4 = baked_density(in.position.xy, close_light, cloud_sampler, cloud);
    float _e5 = smoothed_level(0.0, _e4, cloud);
    metal::float4 _e8 = clouded(_e5, in.position.xy, lut, close_light, wide_light, cloud_sampler, cloud);
    metal::float3 _e10 = linear_from_gamma_rgb(_e8.xyz);
    return fs_cloud_backdrop_linearOutput { metal::float4(_e10, 1.0) };
}
