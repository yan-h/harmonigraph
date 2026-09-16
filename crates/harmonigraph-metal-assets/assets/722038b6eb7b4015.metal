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
    float scale_size;
    float scale_refract;
    float scale_relief;
    float scale_glint;
    float cloud_ambient;
    float _pad2_;
};
struct Pile {
    float height;
    char _pad1[4];
    metal::float2 slope;
};
constant metal::float2x2 CLOUD_TURN = metal::float2x2(metal::float2(0.8, 0.6), metal::float2(-0.6, 0.8));
constant float DOME_RADIUS = 1.15;
constant float DOME_JITTER = 0.75;
constant float DOME_UNION = 9.0;
constant float DOME_PEAK_SLOPE = 1.5;
constant float DOME_LACUNARITY = 2.1;
constant float DOME_FINE_GAIN = 0.22;

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

metal::float3 cloud_hash3_(
    metal::int2 cell_1
) {
    uint n_1 = {};
    n_1 = (as_type<uint>(cell_1.x) * 2654435769u) ^ (as_type<uint>(cell_1.y) * 2246822507u);
    uint _e11 = n_1;
    uint _e12 = n_1;
    n_1 = (_e11 ^ (_e12 >> 16u)) * 2146121005u;
    uint _e18 = n_1;
    uint _e19 = n_1;
    n_1 = (_e18 ^ (_e19 >> 15u)) * 2221713035u;
    uint _e25 = n_1;
    uint _e26 = n_1;
    n_1 = _e25 ^ (_e26 >> 16u);
    uint _e30 = n_1;
    uint _e36 = n_1;
    uint _e44 = n_1;
    return metal::float3(static_cast<float>(_e30 & 1023u) / 1023.0, static_cast<float>((_e36 >> 10u) & 1023u) / 1023.0, static_cast<float>((_e44 >> 20u) & 1023u) / 1023.0);
}

Pile dome_octave(
    metal::float2 r,
    float occupancy
) {
    float weight = 0.0;
    metal::float2 slope = metal::float2(0.0);
    int j = -1;
    int i_1 = {};
    Pile out = {};
    metal::float2 base_2 = metal::floor(r);
    uint2 loop_bound_1 = uint2(4294967295u);
    bool loop_init_1 = true;
    while(true) {
        if (metal::all(loop_bound_1 == uint2(0u))) { break; }
        loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
        if (!loop_init_1) {
            int _e70 = j;
            j = as_type<int>(as_type<uint>(_e70) + as_type<uint>(1));
        }
        loop_init_1 = false;
        int _e10 = j;
        if (_e10 <= 1) {
        } else {
            break;
        }
        {
            i_1 = -1;
            uint2 loop_bound_2 = uint2(4294967295u);
            bool loop_init_2 = true;
            while(true) {
                if (metal::all(loop_bound_2 == uint2(0u))) { break; }
                loop_bound_2 -= uint2(loop_bound_2.y == 0u, 1u);
                if (!loop_init_2) {
                    int _e67 = i_1;
                    i_1 = as_type<int>(as_type<uint>(_e67) + as_type<uint>(1));
                }
                loop_init_2 = false;
                int _e15 = i_1;
                if (_e15 <= 1) {
                } else {
                    break;
                }
                {
                    int _e19 = i_1;
                    int _e20 = j;
                    metal::int2 cell_2 = as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(base_2)) + as_type<metal::uint2>(metal::int2(_e19, _e20)));
                    metal::float3 _e23 = cloud_hash3_(cell_2);
                    if (_e23.z >= occupancy) {
                        continue;
                    }
                    int _e26 = i_1;
                    int _e28 = j;
                    metal::float2 centre = ((base_2 + metal::float2(static_cast<float>(_e26), static_cast<float>(_e28))) + metal::float2(0.5)) + ((_e23.xy - metal::float2(0.5)) * DOME_JITTER);
                    metal::float2 d = (r - centre) / metal::float2(1.15);
                    float q_1 = 1.0 - metal::dot(d, d);
                    if (q_1 <= 0.0) {
                        continue;
                    }
                    float root = metal::sqrt(q_1);
                    float h = q_1 * root;
                    float w = metal::exp(DOME_UNION * h);
                    float _e56 = weight;
                    weight = _e56 + w;
                    metal::float2 _e58 = slope;
                    slope = _e58 + (w * (((-3.0 * root) * d) / metal::float2(1.15)));
                }
            }
        }
    }
    float _e74 = weight;
    if (_e74 <= 0.0) {
        out.height = 0.0;
        out.slope = metal::float2(0.0);
        Pile _e82 = out;
        return _e82;
    }
    float _e84 = weight;
    out.height = metal::log(_e84) / DOME_UNION;
    metal::float2 _e89 = slope;
    float _e90 = weight;
    out.slope = _e89 / metal::float2(_e90);
    Pile _e93 = out;
    return _e93;
}

Pile cloud_domes(
    metal::float2 r_1,
    float occupancy_1
) {
    Pile out_1 = {};
    Pile _e2 = dome_octave(r_1, occupancy_1);
    Pile _e9 = dome_octave((r_1 * DOME_LACUNARITY) + metal::float2(17.3, 5.9), occupancy_1);
    out_1.height = (_e2.height + (DOME_FINE_GAIN * _e9.height)) / 1.22;
    out_1.slope = (_e2.slope + (0.46199998 * _e9.slope)) / metal::float2(1.22);
    Pile _e27 = out_1;
    return _e27;
}

float cloud_light(
    metal::float2 pt,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    metal::float2 _e3 = cloud.size;
    metal::float2 uv_1 = pt / _e3;
    metal::float4 _e8 = wide_light.sample(cloud_sampler, uv_1, metal::level(0.0));
    float _e10 = density_decode(_e8.x);
    metal::float4 _e14 = close_light.sample(cloud_sampler, uv_1, metal::level(0.0));
    float material_1 = _e14.x;
    return 1.4 * metal::max(_e10, 0.85 * material_1);
}

metal::float3 softened(
    metal::float3 colour
) {
    float m = metal::max(metal::max(colour.x, colour.y), colour.z);
    if (m <= 0.75) {
        return metal::max(colour, metal::float3(0.0));
    }
    return metal::max(colour, metal::float3(0.0)) * ((0.75 + (0.25 * (1.0 - metal::exp((0.75 - m) * 4.0)))) / m);
}

float cloud_billow(
    metal::float2 q
) {
    float _e5 = cloud_fbm(q * 0.7, 2, 0.5);
    float _e14 = cloud_fbm((q * 0.7) + metal::float2(8.3, 2.7), 2, 0.5);
    metal::float2 warp = 0.7 * metal::float2(_e5, _e14);
    float _e21 = cloud_fbm(q + warp, 4, 0.5);
    return 0.5 + (0.5 * _e21);
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
    metal::float2 pt_1 = (position_1 / metal::float2(_e21)) - _e26;
    float _e30 = cloud.cloud_scale;
    float units = 5.0 / _e30;
    metal::float2 _e35 = cloud.size;
    float _e42 = cloud.size.y;
    metal::float2 _e48 = cloud.drift;
    metal::float2 q_2 = (((pt_1 - (_e35 * 0.5)) / metal::float2(_e42)) * units) + _e48;
    float _e52 = cloud.cloud_cover;
    float low = 0.62 - (0.4 * _e52);
    float _e59 = cloud_billow(q_2);
    float density = metal::smoothstep(low, low + 0.3, _e59);
    if (density <= 0.003) {
        return base;
    }
    float _e65 = cloud.scale_size;
    float scale_units = 6.0 / _e65;
    float _e71 = cloud.size.y;
    float scale_points = (_e71 / units) / scale_units;
    Pile _e76 = cloud_domes(q_2 * scale_units, 0.78);
    metal::float2 face = _e76.slope / metal::float2(1.5);
    float _e83 = cloud.scale_refract;
    float bend = (_e83 * scale_points) * density;
    float _e88 = cloud_light(pt_1 - (face * bend), close_light, wide_light, cloud_sampler, cloud);
    metal::float2 reach = metal::float2(scale_points * 0.75, 0.0);
    float _e95 = cloud_light(pt_1 + reach.xy, close_light, wide_light, cloud_sampler, cloud);
    float _e98 = cloud_light(pt_1 - reach.xy, close_light, wide_light, cloud_sampler, cloud);
    float _e102 = cloud_light(pt_1 + reach.yx, close_light, wide_light, cloud_sampler, cloud);
    float _e105 = cloud_light(pt_1 - reach.yx, close_light, wide_light, cloud_sampler, cloud);
    metal::float2 grad = metal::float2(_e95 - _e98, _e102 - _e105);
    float magnitude = metal::length(grad);
    float directed = metal::smoothstep(0.0, 0.03, magnitude);
    metal::float2 toward = metal::normalize(metal::mix(metal::float2(0.55, -0.83), grad / metal::float2(metal::max(magnitude, 0.00001)), directed));
    float aimed = 0.5 + (0.5 * directed);
    float _e127 = cloud.scale_relief;
    float relief = _e127 * density;
    metal::float3 normal = metal::normalize(metal::float3(-(face) * relief, 1.0));
    metal::float3 sun = metal::normalize(metal::float3(toward * 0.7, 0.7));
    float diffuse = metal::max(metal::dot(normal, sun), 0.0) / sun.z;
    metal::float3 half_ = metal::normalize(sun + metal::float3(0.0, 0.0, 1.0));
    float flat_glint = metal::pow(half_.z, 14.0);
    float glint = (metal::max(metal::pow(metal::max(metal::dot(normal, half_), 0.0), 14.0) - flat_glint, 0.0) / (1.0 - flat_glint)) * aimed;
    float _e170 = cloud_billow(q_2 + (toward * 0.18));
    float ahead = metal::smoothstep(low, low + 0.3, _e170);
    float rim = metal::clamp((density - ahead) * 2.5, 0.0, 1.0) * aimed;
    float shade = 1.2 - (0.4 * density);
    float _e192 = cloud.cloud_ambient;
    float lit = (((_e88 * diffuse) * shade) * (0.8 + (0.6 * rim))) + _e192;
    metal::float3 _e197 = palette_color(metal::clamp(lit, 0.0, 1.0), lut);
    float _e200 = cloud.scale_glint;
    metal::float3 body = _e197 + metal::float3(((glint * _e200) * _e88) * 0.5);
    metal::float3 _e207 = softened(body);
    float _e210 = cloud.cloud_depth;
    return metal::mix(base, _e207, _e210 * density);
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
