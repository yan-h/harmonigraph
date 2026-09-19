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
    float scale_size;
    float scale_variety;
    float scale_refract;
    float scale_relief;
    float scale_facet;
    float scale_rock;
    uint cloud_style;
    float wash_size;
    float wash_variety;
    float wash_fuzz;
    float wash_ragged;
    float wash_lobe;
    float wash_refract;
    float wash_pool;
    float wash_grain;
    float wash_layers;
    float wash_soften;
    float wash_wander;
    float wash_black;
    char _pad31[4];
};
struct Pile {
    metal::float2 face;
    metal::float2 to_centre;
    metal::float2 rock;
};
struct Glob {
    metal::float2 centre;
    float edge;
    float order;
};
struct Wash {
    metal::float2 centre;
    metal::float2 under;
    metal::float2 front;
    float near;
    float edge;
    float cover;
    float tau;
};
struct Painted {
    float tone;
    float hold;
};
constant float CLOUD_UNITS = 10.0;
constant float SCALE_CELLS = 2.7272727;
constant float DOME_RADIUS = 1.15;
constant float DOME_JITTER = 0.3;
constant float DOME_RADIUS_MIN = 0.95;
constant float DOME_RADIUS_MAX = 1.32;
constant float DOME_UNION = 9.0;
constant float DOME_VARIETY_GAIN = 5.0;
constant float DOME_FACE = 1.5122874;
constant float ROCK_TILT = 0.3;
constant float SUN_LEAN = 1.0;
constant float SUN_KNEE = 0.03;
constant float RELIEF_FLOOR_FALL = 3.22;
constant float DOME_LACUNARITY = 2.1;
constant float DOME_FINE_GAIN = 0.22;
constant float CLOUD_SHADE = 0.64;
constant int WASH_RING = 2;
constant float WASH_JITTER = 0.4;
constant float WASH_RAGGED = 0.3;
constant float WASH_RADIUS = 1.18;
constant float WASH_RADIUS_MIN = 1.02;
constant float WASH_RADIUS_MAX = 1.66;
constant float WASH_CELLS = 5.25;
constant float WASH_LACUNARITY = 2.1;
constant float WASH_FINE_OCCUPANCY = 0.2;
constant float WASH_WARP = 0.45;
constant float WASH_WARP_SCALE = 0.9;
constant float WASH_RAGGED_SCALE = 2.8;
constant float WASH_POOL = 0.44;
constant float WASH_GRAIN = 0.1;
constant float WASH_POOL_WIDTH = 0.55;
constant float WASH_SURF = 0.07;
constant float WASH_PIG_DEPTH = 0.35;
constant float WASH_TONE_FLOOR = 0.05;
constant float WASH_PIVOT = 0.45;
constant float WASH_LIFT_A = 1.15;
constant float WASH_LIFT_B = 0.16;
constant float WASH_BLACK_KNEE = 0.35;

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

metal::float4 cloud_hash4_(
    metal::int2 cell
) {
    uint n = {};
    uint m = {};
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
    m = (_e30 ^ 3039394381u) * 1759714724u;
    uint _e36 = m;
    uint _e37 = m;
    m = _e36 ^ (_e37 >> 15u);
    uint _e41 = n;
    uint _e47 = n;
    uint _e55 = n;
    uint _e63 = m;
    return metal::float4(static_cast<float>(_e41 & 1023u) / 1023.0, static_cast<float>((_e47 >> 10u) & 1023u) / 1023.0, static_cast<float>((_e55 >> 20u) & 1023u) / 1023.0, static_cast<float>(_e63 & 1023u) / 1023.0);
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
    float gain = {};
    Pile out = {};
    metal::float2 base_2 = metal::floor(r);
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            int _e144 = j;
            j = as_type<int>(as_type<uint>(_e144) + as_type<uint>(1));
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
                    int _e141 = i;
                    i = as_type<int>(as_type<uint>(_e141) + as_type<uint>(1));
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
                    metal::int2 cell_3 = as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(base_2)) + as_type<metal::uint2>(metal::int2(_e24, _e25)));
                    metal::float4 _e28 = cloud_hash4_(cell_3);
                    int _e29 = i;
                    int _e31 = j;
                    metal::float2 centre = ((base_2 + metal::float2(static_cast<float>(_e29), static_cast<float>(_e31))) + metal::float2(0.5)) + ((_e28.xy - metal::float2(0.5)) * DOME_JITTER);
                    float _e52 = cloud.scale_variety;
                    float radius = metal::mix(DOME_RADIUS, metal::mix(DOME_RADIUS_MIN, DOME_RADIUS_MAX, _e28.z), _e52);
                    metal::float2 d = (r - centre) / metal::float2(radius);
                    float q = 1.0 - metal::dot(d, d);
                    if (q <= 0.0) {
                        continue;
                    }
                    float root = metal::sqrt(q);
                    float h = q * root;
                    gain = 1.0;
                    float _e68 = cloud.scale_variety;
                    if (_e68 > 0.0) {
                        float _e74 = cloud.scale_variety;
                        gain = metal::exp2((DOME_VARIETY_GAIN * _e74) * ((2.0 * _e28.w) - 1.0));
                    }
                    float _e83 = gain;
                    float w = _e83 * (metal::exp(DOME_UNION * h) - 1.0);
                    float _e90 = weight;
                    weight = _e90 + w;
                    metal::float2 _e92 = face;
                    face = _e92 + ((w * -(((DOME_FACE * root) * radius))) * d);
                    metal::float2 _e100 = to_centre;
                    to_centre = _e100 + (w * (centre - r));
                    float _e106 = cloud.scale_rock;
                    if (_e106 > 0.0) {
                        metal::float2 _e109 = rock;
                        float _e112 = cloud.time;
                        float _e126 = cloud.time;
                        rock = _e109 + (w * metal::float2(metal::sin((_e112 * (0.2 + (0.3 * _e28.x))) + (_e28.y * 6.2831855)), metal::cos((_e126 * (0.25 + (0.2 * _e28.y))) + (_e28.x * 6.2831855))));
                    }
                }
            }
        }
    }
    float _e148 = weight;
    if (_e148 <= 0.0) {
        out.face = metal::float2(0.0);
        out.to_centre = metal::float2(0.0);
        out.rock = metal::float2(0.0);
        Pile _e160 = out;
        return _e160;
    }
    metal::float2 _e162 = face;
    float _e163 = weight;
    out.face = _e162 / metal::float2(_e163);
    metal::float2 _e167 = to_centre;
    float _e168 = weight;
    out.to_centre = _e167 / metal::float2(_e168);
    metal::float2 _e172 = rock;
    float _e173 = weight;
    out.rock = _e172 / metal::float2(_e173);
    Pile _e176 = out;
    return _e176;
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
    metal::float2 pt_3 = (position_1 / metal::float2(_e21)) - _e26;
    metal::float2 _e30 = cloud.size;
    float _e37 = cloud.size.y;
    metal::float2 _e44 = cloud.drift;
    metal::float2 q_1 = (((pt_3 - (_e30 * 0.5)) / metal::float2(_e37)) * CLOUD_UNITS) + _e44;
    float _e49 = cloud.scale_size;
    float scale_units = SCALE_CELLS / _e49;
    float _e54 = cloud.size.y;
    float scale_points = (_e54 / CLOUD_UNITS) / scale_units;
    Pile _e59 = cloud_domes(q_1 * scale_units, cloud);
    metal::float2 face_1 = _e59.face;
    float _e63 = cloud.scale_refract;
    float bend = _e63 * scale_points;
    float _e71 = cloud.scale_facet;
    metal::float2 lookup = metal::mix(-(face_1) * bend, _e59.to_centre * scale_points, _e71);
    float _e74 = cloud_light(pt_3 + lookup, close_light, wide_light, cloud_sampler, cloud);
    metal::float2 reach = metal::float2(scale_points * 0.75, 0.0);
    float _e81 = cloud_light(pt_3 + reach.xy, close_light, wide_light, cloud_sampler, cloud);
    float _e84 = cloud_light(pt_3 - reach.xy, close_light, wide_light, cloud_sampler, cloud);
    float _e88 = cloud_light(pt_3 + reach.yx, close_light, wide_light, cloud_sampler, cloud);
    float _e91 = cloud_light(pt_3 - reach.yx, close_light, wide_light, cloud_sampler, cloud);
    metal::float2 grad = metal::float2(_e81 - _e84, _e88 - _e91);
    metal::float2 lean = grad * (SUN_LEAN / (SUN_KNEE + metal::length(grad)));
    float relief = cloud.scale_relief;
    tilt = face_1;
    float _e106 = cloud.scale_rock;
    if (_e106 > 0.0) {
        metal::float2 _e109 = tilt;
        float _e113 = cloud.scale_rock;
        tilt = _e109 + (_e59.rock * (_e113 * ROCK_TILT));
    }
    metal::float2 _e118 = tilt;
    metal::float3 normal = metal::normalize(metal::float3(-(_e118) * relief, 1.0));
    metal::float3 sun = metal::normalize(metal::float3(lean, 1.0));
    float lambert = metal::max(metal::dot(normal, sun), 0.0) / sun.z;
    float diffuse = metal::mix(metal::pow(1.0 - relief, RELIEF_FLOOR_FALL), 1.0, lambert);
    float lit = (_e74 * diffuse) * CLOUD_SHADE;
    metal::float3 _e144 = palette_color(metal::clamp(lit, 0.0, 1.0), lut);
    float _e147 = cloud.cloud_depth;
    return metal::mix(base, _e144, _e147);
}

metal::float3 wash_hash(
    metal::int2 cell_1,
    uint salt
) {
    uint n_1 = {};
    n_1 = (as_type<uint>(cell_1.x) * 2654435769u) ^ (as_type<uint>(cell_1.y) * 2246822507u);
    uint _e12 = n_1;
    n_1 = _e12 ^ (salt * 668265261u);
    uint _e16 = n_1;
    uint _e17 = n_1;
    n_1 = (_e16 ^ (_e17 >> 16u)) * 2146121005u;
    uint _e23 = n_1;
    uint _e24 = n_1;
    n_1 = (_e23 ^ (_e24 >> 15u)) * 2221713035u;
    uint _e30 = n_1;
    uint _e31 = n_1;
    n_1 = _e30 ^ (_e31 >> 16u);
    uint _e35 = n_1;
    uint _e41 = n_1;
    uint _e49 = n_1;
    return metal::float3(static_cast<float>(_e35 & 1023u) / 1023.0, static_cast<float>((_e41 >> 10u) & 1023u) / 1023.0, static_cast<float>((_e49 >> 20u) & 1023u) / 1023.0);
}

float wash_noise(
    metal::float2 p,
    uint salt_1
) {
    metal::float2 b_1 = metal::floor(p);
    metal::float2 f_1 = p - b_1;
    metal::float2 t = (f_1 * f_1) * (metal::float2(3.0) - (2.0 * f_1));
    metal::int2 i_3 = naga_f2i32(b_1);
    metal::float3 _e12 = wash_hash(i_3, salt_1);
    float n00_ = _e12.x;
    metal::float3 _e18 = wash_hash(as_type<metal::int2>(as_type<metal::uint2>(i_3) + as_type<metal::uint2>(metal::int2(1, 0))), salt_1);
    float n10_ = _e18.x;
    metal::float3 _e24 = wash_hash(as_type<metal::int2>(as_type<metal::uint2>(i_3) + as_type<metal::uint2>(metal::int2(0, 1))), salt_1);
    float n01_ = _e24.x;
    metal::float3 _e30 = wash_hash(as_type<metal::int2>(as_type<metal::uint2>(i_3) + as_type<metal::uint2>(metal::int2(1, 1))), salt_1);
    float n11_ = _e30.x;
    return metal::mix(metal::mix(n00_, n10_, t.x), metal::mix(n01_, n11_, t.x), t.y);
}

float wash_fbm(
    metal::float2 p_1,
    uint salt_2
) {
    float _e2 = wash_noise(p_1, salt_2);
    float _e11 = wash_noise((p_1 * 2.07) + metal::float2(13.1, -7.3), salt_2 + 31u);
    return (_e2 + (0.5 * _e11)) / 1.5;
}

Glob wash_glob(
    metal::int2 cell_2,
    uint salt_3,
    metal::float2 r_2,
    float wob,
    float occupancy,
    constant Cloud& cloud
) {
    metal::float2 offset = {};
    Glob out_2 = {};
    metal::float3 _e5 = wash_hash(cell_2, salt_3);
    metal::float3 _e8 = wash_hash(cell_2, salt_3 + 77u);
    offset = (_e5.xy - metal::float2(0.5)) * WASH_JITTER;
    float _e18 = cloud.wash_wander;
    if (_e18 > 0.0) {
        float _e23 = cloud.time;
        float _e32 = cloud.wash_wander;
        float turn = (_e23 * (0.05 + (0.12 * _e8.z))) * _e32;
        float c = metal::cos(turn);
        float s = metal::sin(turn);
        float _e37 = offset.x;
        float _e40 = offset.y;
        float _e44 = offset.x;
        float _e47 = offset.y;
        offset = metal::float2((_e37 * c) - (_e40 * s), (_e44 * s) + (_e47 * c));
    }
    metal::float2 _e55 = offset;
    metal::float2 centre_1 = (static_cast<metal::float2>(cell_2) + metal::float2(0.5)) + _e55;
    float _e64 = cloud.wash_variety;
    float radius_1 = metal::mix(WASH_RADIUS, metal::mix(WASH_RADIUS_MIN, WASH_RADIUS_MAX, _e5.z), _e64);
    out_2.centre = centre_1;
    out_2.order = _e8.x;
    bool present = _e8.y < occupancy;
    out_2.edge = present ? ((metal::length(r_2 - centre_1) / radius_1) + wob) : 1000000000.0;
    Glob _e79 = out_2;
    return _e79;
}

Wash wash_scan(
    metal::float2 r_3,
    uint salt_4,
    float occupancy_1,
    float wob_1,
    constant Cloud& cloud
) {
    Wash out_3 = {};
    float best = -1000000000.0;
    float second = -1000000000.0;
    float near_a = -1000000000.0;
    float near_b = -1000000000.0;
    float order_a = -1000000000.0;
    float order_b = -1000000000.0;
    metal::float2 front_a = {};
    metal::float2 front_b = {};
    int j_1 = -2;
    int i_1 = {};
    out_3.centre = r_3;
    out_3.under = r_3;
    out_3.front = r_3;
    out_3.near = -1000000000.0;
    out_3.edge = 1.0;
    out_3.cover = 0.0;
    out_3.tau = 0.0;
    front_a = r_3;
    front_b = r_3;
    metal::int2 base_3 = naga_f2i32(metal::floor(r_3));
    uint2 loop_bound_2 = uint2(4294967295u);
    bool loop_init_2 = true;
    while(true) {
        if (metal::all(loop_bound_2 == uint2(0u))) { break; }
        loop_bound_2 -= uint2(loop_bound_2.y == 0u, 1u);
        if (!loop_init_2) {
            int _e108 = j_1;
            j_1 = as_type<int>(as_type<uint>(_e108) + as_type<uint>(1));
        }
        loop_init_2 = false;
        int _e34 = j_1;
        if (_e34 <= WASH_RING) {
        } else {
            break;
        }
        {
            i_1 = -2;
            uint2 loop_bound_3 = uint2(4294967295u);
            bool loop_init_3 = true;
            while(true) {
                if (metal::all(loop_bound_3 == uint2(0u))) { break; }
                loop_bound_3 -= uint2(loop_bound_3.y == 0u, 1u);
                if (!loop_init_3) {
                    int _e105 = i_1;
                    i_1 = as_type<int>(as_type<uint>(_e105) + as_type<uint>(1));
                }
                loop_init_3 = false;
                int _e39 = i_1;
                if (_e39 <= WASH_RING) {
                } else {
                    break;
                }
                {
                    int _e42 = i_1;
                    int _e43 = j_1;
                    Glob _e46 = wash_glob(as_type<metal::int2>(as_type<metal::uint2>(base_3) + as_type<metal::uint2>(metal::int2(_e42, _e43))), salt_4, r_3, wob_1, occupancy_1, cloud);
                    float prox = 1.0 - _e46.edge;
                    float _e52 = out_3.cover;
                    out_3.cover = metal::max(_e52, metal::clamp(prox / 0.05, 0.0, 1.0));
                    float body = metal::clamp(prox / 0.1, 0.0, 1.0);
                    float _e65 = out_3.tau;
                    out_3.tau = _e65 + ((body * body) * (3.0 - (2.0 * body)));
                    if (_e46.edge < 1.0) {
                        float _e77 = best;
                        if (_e46.order > _e77) {
                            float _e79 = best;
                            second = _e79;
                            metal::float2 _e82 = out_3.centre;
                            out_3.under = _e82;
                            best = _e46.order;
                            out_3.centre = _e46.centre;
                            out_3.edge = _e46.edge;
                        } else {
                            float _e89 = second;
                            if (_e46.order > _e89) {
                                second = _e46.order;
                                out_3.under = _e46.centre;
                            }
                        }
                    } else {
                        float _e94 = near_a;
                        if (prox > _e94) {
                            float _e96 = near_a;
                            near_b = _e96;
                            float _e97 = order_a;
                            order_b = _e97;
                            metal::float2 _e98 = front_a;
                            front_b = _e98;
                            near_a = prox;
                            order_a = _e46.order;
                            front_a = _e46.centre;
                        } else {
                            float _e101 = near_b;
                            if (prox > _e101) {
                                near_b = prox;
                                order_b = _e46.order;
                                front_b = _e46.centre;
                            }
                        }
                    }
                }
            }
        }
    }
    float _e111 = order_b;
    float _e112 = best;
    if (_e111 > _e112) {
        float _e115 = near_b;
        out_3.near = _e115;
        metal::float2 _e117 = front_b;
        out_3.front = _e117;
    }
    float _e118 = order_a;
    float _e119 = best;
    if (_e118 > _e119) {
        float _e122 = near_a;
        out_3.near = _e122;
        metal::float2 _e124 = front_a;
        out_3.front = _e124;
    }
    Wash _e125 = out_3;
    return _e125;
}

float wash_light(
    metal::float2 pt_1,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    metal::float2 _e3 = cloud.size;
    metal::float2 uv_2 = pt_1 / _e3;
    metal::float4 _e8 = close_light.sample(cloud_sampler, uv_2, metal::level(0.0));
    float material_2 = _e8.x;
    float _e12 = cloud.wash_soften;
    if (_e12 <= 0.0) {
        return material_2;
    }
    metal::float4 _e18 = wide_light.sample(cloud_sampler, uv_2, metal::level(0.0));
    float _e20 = density_decode(_e18.x);
    float _e23 = cloud.wash_soften;
    return metal::mix(material_2, _e20, _e23);
}

Painted wash_tone(
    Wash f,
    metal::float2 r_4,
    float pane_per_cell,
    metal::float2 pt_2,
    float average_pile,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    metal::float2 look = {};
    float fa = {};
    float bl = {};
    float pigment = {};
    float hold = 1.0;
    float fuzz = cloud.wash_fuzz;
    float feather = 0.1 + (0.8 * fuzz);
    float bleed = 0.12 + (0.78 * fuzz);
    float _e19 = cloud.wash_pool;
    float tide = (WASH_POOL * _e19) * (1.0 - (0.75 * fuzz));
    float surf = WASH_SURF * (1.0 - (0.45 * fuzz));
    look = f.centre;
    fa = metal::clamp((f.edge - (1.0 - feather)) / feather, 0.0, 1.0);
    float _e43 = fa;
    float _e44 = fa;
    float _e46 = fa;
    fa = ((_e43 * _e44) * (3.0 - (2.0 * _e46))) * 0.5;
    metal::float2 _e54 = look;
    float _e56 = fa;
    look = metal::mix(_e54, f.under, _e56);
    bl = metal::clamp((f.near + bleed) / bleed, 0.0, 1.0);
    float _e65 = bl;
    float _e66 = bl;
    float _e68 = bl;
    bl = ((_e65 * _e66) * (3.0 - (2.0 * _e68))) * 0.5;
    metal::float2 _e76 = look;
    float _e78 = bl;
    look = metal::mix(_e76, f.front, _e78);
    metal::float2 _e80 = look;
    float _e85 = cloud.wash_refract;
    float _e88 = wash_light(pt_2 + (((_e80 - r_4) * pane_per_cell) * _e85), close_light, wide_light, cloud_sampler, cloud);
    float paper = metal::clamp((WASH_PIVOT + (WASH_LIFT_A * (_e88 - WASH_PIVOT))) + WASH_LIFT_B, 0.0, 1.0);
    float rim = metal::clamp(f.edge, 0.0, 1.0);
    pigment = (surf * rim) * rim;
    float crescent = metal::clamp((f.near + WASH_POOL_WIDTH) / WASH_POOL_WIDTH, 0.0, 1.0);
    float _e115 = pigment;
    pigment = _e115 + ((tide * crescent) * crescent);
    float _e119 = pigment;
    float _e123 = cloud.wash_grain;
    pigment = _e119 + ((WASH_GRAIN * _e123) * metal::max(f.tau - average_pile, 0.0));
    float _e131 = pigment;
    float pig = metal::max(_e131, 0.0);
    float tone = paper - (pig * (WASH_PIG_DEPTH + (0.65 * paper)));
    float _e144 = cloud.wash_black;
    if (_e144 > 0.0) {
        float _e150 = cloud.wash_black;
        hold = metal::smoothstep(0.0, WASH_BLACK_KNEE * _e150, _e88);
    }
    float _e154 = hold;
    return Painted {tone, _e154};
}

float wash_average_pile(
    float occupancy_2,
    constant Cloud& cloud
) {
    float _e5 = cloud.wash_variety;
    float lo = metal::mix(WASH_RADIUS, WASH_RADIUS_MIN, _e5);
    float _e11 = cloud.wash_variety;
    float hi = metal::mix(WASH_RADIUS, WASH_RADIUS_MAX, _e11);
    return ((occupancy_2 * 3.1415927) * (((lo * lo) + (lo * hi)) + (hi * hi))) / 3.0;
}

metal::float3 wash_clouds(
    metal::float3 base_1,
    metal::float2 position_2,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    bool local_1 = {};
    metal::float2 warped = {};
    float wob_2 = 0.0;
    Painted paint = {};
    float _e4 = cloud.cloud_depth;
    if (!((_e4 <= 0.0))) {
        metal::float2 _e12 = cloud.step;
        local_1 = metal::all(_e12 == metal::float2(0.0));
    } else {
        local_1 = true;
    }
    bool _e18 = local_1;
    if (_e18) {
        return base_1;
    }
    float _e21 = cloud.ppp;
    metal::float2 _e26 = cloud.origin;
    metal::float2 pt_4 = (position_2 / metal::float2(_e21)) - _e26;
    metal::float2 _e30 = cloud.size;
    float _e37 = cloud.size.y;
    metal::float2 _e44 = cloud.drift;
    metal::float2 q_2 = (((pt_4 - (_e30 * 0.5)) / metal::float2(_e37)) * CLOUD_UNITS) + _e44;
    float _e49 = cloud.wash_size;
    float cells = WASH_CELLS / _e49;
    float _e54 = cloud.size.y;
    float pane_per_cell_1 = (_e54 / CLOUD_UNITS) / cells;
    metal::float2 r_5 = q_2 * cells;
    warped = r_5;
    float _e62 = cloud.wash_lobe;
    if (_e62 > 0.0) {
        float _e68 = cloud.wash_lobe;
        float amp = WASH_WARP * _e68;
        metal::float2 _e70 = warped;
        float _e76 = wash_fbm(r_5 * WASH_WARP_SCALE, 71u);
        float _e86 = wash_fbm((r_5 * WASH_WARP_SCALE) + metal::float2(37.0, -19.0), 73u);
        warped = _e70 + ((amp * 2.0) * metal::float2(_e76 - 0.5, _e86 - 0.5));
    }
    float _e96 = cloud.wash_ragged;
    if (_e96 > 0.0) {
        float _e102 = cloud.wash_ragged;
        float _e107 = wash_fbm(r_5 * WASH_RAGGED_SCALE, 41u);
        wob_2 = (WASH_RAGGED * _e102) * (_e107 - 1.0);
    }
    metal::float2 _e111 = warped;
    float _e114 = wob_2;
    Wash _e115 = wash_scan(_e111, 1u, 1.0, _e114, cloud);
    metal::float2 _e116 = warped;
    float _e118 = wash_average_pile(1.0, cloud);
    Painted _e119 = wash_tone(_e115, _e116, pane_per_cell_1, pt_4, _e118, close_light, wide_light, cloud_sampler, cloud);
    paint = _e119;
    float _e123 = cloud.wash_layers;
    if (_e123 > 0.0) {
        metal::float2 _e126 = warped;
        metal::float2 fine_r = (_e126 * WASH_LACUNARITY) + metal::float2(17.3, 5.9);
        float _e135 = wob_2;
        Wash _e136 = wash_scan(fine_r, 2u, WASH_FINE_OCCUPANCY, _e135, cloud);
        float _e140 = wash_average_pile(WASH_FINE_OCCUPANCY, cloud);
        Painted _e141 = wash_tone(_e136, fine_r, pane_per_cell_1 / WASH_LACUNARITY, pt_4, _e140, close_light, wide_light, cloud_sampler, cloud);
        float _e144 = cloud.wash_layers;
        float over = _e144 * _e136.cover;
        float _e149 = paint.tone;
        paint.tone = metal::mix(_e149, _e141.tone, over);
        float _e154 = paint.hold;
        paint.hold = metal::mix(_e154, _e141.hold, over);
    }
    float _e158 = paint.tone;
    float _e163 = paint.hold;
    metal::float3 _e165 = palette_color(metal::clamp(_e158, WASH_TONE_FLOOR, 1.0) * _e163, lut);
    float _e168 = cloud.cloud_depth;
    return metal::mix(base_1, _e165, _e168);
}

metal::float4 clouded(
    float level_2,
    metal::float2 position_3,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    metal::float4 _e2 = density_color(level_2, lut, cloud);
    uint _e5 = cloud.cloud_style;
    if (_e5 == 1u) {
        metal::float3 _e9 = wash_clouds(_e2.xyz, position_3, lut, close_light, wide_light, cloud_sampler, cloud);
        return metal::float4(_e9, 1.0);
    }
    metal::float3 _e13 = scale_clouds(_e2.xyz, position_3, lut, close_light, wide_light, cloud_sampler, cloud);
    return metal::float4(_e13, 1.0);
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
, metal::texture2d<float, metal::access::sample> wide_light [[texture(2)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(2)]]
) {
    const VertexOut in = { position_4, varyings.slab, varyings.t };
    float _e4 = baked_density(in.position.xy, close_light, cloud_sampler, cloud);
    float _e5 = smoothed_level(0.0, _e4, cloud);
    metal::float4 _e8 = clouded(_e5, in.position.xy, lut, close_light, wide_light, cloud_sampler, cloud);
    metal::float3 _e10 = linear_from_gamma_rgb(_e8.xyz);
    return fs_cloud_backdrop_linearOutput { metal::float4(_e10, 1.0) };
}
