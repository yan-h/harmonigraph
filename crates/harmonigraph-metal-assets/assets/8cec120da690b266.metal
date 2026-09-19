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
    float scale_size;
    float scale_variety;
    float scale_refract;
    float scale_relief;
    float scale_shade_floor;
    float scale_facet;
    float scale_rock;
};
struct Pile {
    metal::float2 face;
    metal::float2 to_centre;
    metal::float2 rock;
};
constant float DOME_RADIUS = 1.15;
constant float DOME_JITTER = 0.3;
constant float DOME_RADIUS_MIN = 0.95;
constant float DOME_RADIUS_MAX = 1.32;
constant float DOME_UNION = 9.0;
constant float DOME_FACE = 1.5122874;
constant float ROCK_TILT = 0.3;
constant float SUN_LEAN = 1.0;
constant float SUN_KNEE = 0.03;
constant float DOME_LACUNARITY = 2.1;
constant float DOME_FINE_GAIN = 0.22;
constant float CLOUD_SHADE = 0.64;

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
    uint i_1 = naga_f2u32(metal::clamp(metal::floor(x_1), 0.0, static_cast<float>(levels - 1u)));
    uint clamped_lod_e22 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e22 = lut.read(metal::min(metal::uint2(metal::uint2(i_1, 0u)), metal::uint2(lut.get_width(clamped_lod_e22), lut.get_height(clamped_lod_e22)) - 1), clamped_lod_e22);
    metal::float3 a = _e22.xyz;
    if (x_1 < 0.0) {
        return (a * (x_1 + 0.5)) * 2.0;
    }
    uint clamped_lod_e40 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e40 = lut.read(metal::min(metal::uint2(metal::uint2(metal::min(i_1 + 1u, levels - 1u), 0u)), metal::uint2(lut.get_width(clamped_lod_e40), lut.get_height(clamped_lod_e40)) - 1), clamped_lod_e40);
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

metal::float3 cloud_hash3_(
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
    uint _e36 = n;
    uint _e44 = n;
    return metal::float3(static_cast<float>(_e30 & 1023u) / 1023.0, static_cast<float>((_e36 >> 10u) & 1023u) / 1023.0, static_cast<float>((_e44 >> 20u) & 1023u) / 1023.0);
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

Pile dome_octave(
    metal::float2 r,
    constant Cloud& cloud
) {
    float weight = 0.0;
    metal::float2 face = metal::float2(0.0);
    metal::float2 to_centre = metal::float2(0.0);
    metal::float2 rock = metal::float2(0.0);
    int j = -1;
    int i = {};
    Pile out = {};
    metal::float2 base_1 = metal::floor(r);
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            int _e121 = j;
            j = as_type<int>(as_type<uint>(_e121) + as_type<uint>(1));
        }
        loop_init = false;
        int _e15 = j;
        if (_e15 <= 1) {
        } else {
            break;
        }
        {
            i = -1;
            uint2 loop_bound_1 = uint2(4294967295u);
            bool loop_init_1 = true;
            while(true) {
                if (metal::all(loop_bound_1 == uint2(0u))) { break; }
                loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
                if (!loop_init_1) {
                    int _e118 = i;
                    i = as_type<int>(as_type<uint>(_e118) + as_type<uint>(1));
                }
                loop_init_1 = false;
                int _e20 = i;
                if (_e20 <= 1) {
                } else {
                    break;
                }
                {
                    int _e24 = i;
                    int _e25 = j;
                    metal::int2 cell_1 = as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(base_1)) + as_type<metal::uint2>(metal::int2(_e24, _e25)));
                    metal::float3 _e28 = cloud_hash3_(cell_1);
                    int _e29 = i;
                    int _e31 = j;
                    metal::float2 centre = ((base_1 + metal::float2(static_cast<float>(_e29), static_cast<float>(_e31))) + metal::float2(0.5)) + ((_e28.xy - metal::float2(0.5)) * DOME_JITTER);
                    float _e52 = cloud.scale_variety;
                    float radius = metal::mix(DOME_RADIUS, metal::mix(DOME_RADIUS_MIN, DOME_RADIUS_MAX, _e28.z), _e52);
                    metal::float2 d = (r - centre) / metal::float2(radius);
                    float q = 1.0 - metal::dot(d, d);
                    if (q <= 0.0) {
                        continue;
                    }
                    float root = metal::sqrt(q);
                    float h = q * root;
                    float w = metal::exp(DOME_UNION * h);
                    float _e67 = weight;
                    weight = _e67 + w;
                    metal::float2 _e69 = face;
                    face = _e69 + ((w * -(((DOME_FACE * root) * radius))) * d);
                    metal::float2 _e77 = to_centre;
                    to_centre = _e77 + (w * (centre - r));
                    float _e83 = cloud.scale_rock;
                    if (_e83 > 0.0) {
                        metal::float2 _e86 = rock;
                        float _e89 = cloud.time;
                        float _e103 = cloud.time;
                        rock = _e86 + (w * metal::float2(metal::sin((_e89 * (0.2 + (0.3 * _e28.x))) + (_e28.y * 6.2831855)), metal::cos((_e103 * (0.25 + (0.2 * _e28.y))) + (_e28.x * 6.2831855))));
                    }
                }
            }
        }
    }
    float _e125 = weight;
    if (_e125 <= 0.0) {
        out.face = metal::float2(0.0);
        out.to_centre = metal::float2(0.0);
        out.rock = metal::float2(0.0);
        Pile _e137 = out;
        return _e137;
    }
    metal::float2 _e139 = face;
    float _e140 = weight;
    out.face = _e139 / metal::float2(_e140);
    metal::float2 _e144 = to_centre;
    float _e145 = weight;
    out.to_centre = _e144 / metal::float2(_e145);
    metal::float2 _e149 = rock;
    float _e150 = weight;
    out.rock = _e149 / metal::float2(_e150);
    Pile _e153 = out;
    return _e153;
}

Pile cloud_domes(
    metal::float2 r_1,
    constant Cloud& cloud
) {
    Pile out_1 = {};
    Pile _e1 = dome_octave(r_1, cloud);
    Pile _e8 = dome_octave((r_1 * DOME_LACUNARITY) + metal::float2(17.3, 5.9), cloud);
    out_1.face = (_e1.face + (0.46199998 * _e8.face)) / metal::float2(1.22);
    out_1.to_centre = _e1.to_centre;
    out_1.rock = (_e1.rock + (DOME_FINE_GAIN * _e8.rock)) / metal::float2(1.22);
    Pile _e29 = out_1;
    return _e29;
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
    metal::float2 tilt = {};
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
    metal::float2 q_1 = (((pt_1 - (_e35 * 0.5)) / metal::float2(_e42)) * units) + _e48;
    float _e52 = cloud.scale_size;
    float scale_units = 6.0 / _e52;
    float _e58 = cloud.size.y;
    float scale_points = (_e58 / units) / scale_units;
    Pile _e62 = cloud_domes(q_1 * scale_units, cloud);
    metal::float2 face_1 = _e62.face;
    float _e66 = cloud.scale_refract;
    float bend = _e66 * scale_points;
    float _e74 = cloud.scale_facet;
    metal::float2 lookup = metal::mix(-(face_1) * bend, _e62.to_centre * scale_points, _e74);
    float _e77 = cloud_light(pt_1 + lookup, close_light, wide_light, cloud_sampler, cloud);
    metal::float2 reach = metal::float2(scale_points * 0.75, 0.0);
    float _e84 = cloud_light(pt_1 + reach.xy, close_light, wide_light, cloud_sampler, cloud);
    float _e87 = cloud_light(pt_1 - reach.xy, close_light, wide_light, cloud_sampler, cloud);
    float _e91 = cloud_light(pt_1 + reach.yx, close_light, wide_light, cloud_sampler, cloud);
    float _e94 = cloud_light(pt_1 - reach.yx, close_light, wide_light, cloud_sampler, cloud);
    metal::float2 grad = metal::float2(_e84 - _e87, _e91 - _e94);
    metal::float2 lean = grad * (SUN_LEAN / (SUN_KNEE + metal::length(grad)));
    float relief = cloud.scale_relief;
    tilt = face_1;
    float _e109 = cloud.scale_rock;
    if (_e109 > 0.0) {
        metal::float2 _e112 = tilt;
        float _e116 = cloud.scale_rock;
        tilt = _e112 + (_e62.rock * (_e116 * ROCK_TILT));
    }
    metal::float2 _e121 = tilt;
    metal::float3 normal = metal::normalize(metal::float3(-(_e121) * relief, 1.0));
    metal::float3 sun = metal::normalize(metal::float3(lean, 1.0));
    float lambert = metal::max(metal::dot(normal, sun), 0.0) / sun.z;
    float _e137 = cloud.scale_shade_floor;
    float diffuse = metal::mix(_e137, 1.0, lambert);
    float lit = (_e77 * diffuse) * CLOUD_SHADE;
    metal::float3 _e146 = palette_color(metal::clamp(lit, 0.0, 1.0), lut);
    float _e149 = cloud.cloud_depth;
    return metal::mix(base, _e146, _e149);
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
