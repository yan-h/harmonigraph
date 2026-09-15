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
    float scale_glint;
    float cloud_ambient;
    float _pad2_;
    float _pad3_;
    float _pad4_;
};
struct Facet {
    metal::float2 centre;
    metal::int2 id;
    float seam;
    char _pad3[4];
};

float density_decode(
    float value
) {
    float y = metal::max(value, 0.0);
    return (2.0 * y) / (0.1 + metal::sqrt(0.01 + (3.6 * y)));
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

float cloud_hash(
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

metal::float2 cloud_hash2_(
    metal::int2 cell_1
) {
    float _e1 = cloud_hash(cell_1);
    float _e6 = cloud_hash(as_type<metal::int2>(as_type<metal::uint2>(cell_1) + as_type<metal::uint2>(metal::int2(1013, 727))));
    return metal::float2(_e1, _e6);
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

float cloud_noise(
    metal::float2 p
) {
    metal::int2 cell_2 = naga_f2i32(metal::floor(p));
    metal::float2 f = metal::fract(p);
    metal::float2 w = (f * f) * (metal::float2(3.0) - (2.0 * f));
    float _e11 = cloud_hash(cell_2);
    float _e16 = cloud_hash(as_type<metal::int2>(as_type<metal::uint2>(cell_2) + as_type<metal::uint2>(metal::int2(1, 0))));
    float _e23 = cloud_hash(as_type<metal::int2>(as_type<metal::uint2>(cell_2) + as_type<metal::uint2>(metal::int2(0, 1))));
    float _e28 = cloud_hash(as_type<metal::int2>(as_type<metal::uint2>(cell_2) + as_type<metal::uint2>(metal::int2(1, 1))));
    return metal::mix(metal::mix(_e11, _e16, w.x), metal::mix(_e23, _e28, w.x), w.y);
}

float billows(
    metal::float2 s
) {
    float _e1 = cloud_noise(s);
    float _e10 = cloud_noise((s * 2.03) + metal::float2(5.2, 1.7));
    float _e20 = cloud_noise((s * 4.1) + metal::float2(1.3, 9.1));
    float _e30 = cloud_noise((s * 8.2) + metal::float2(7.8, 3.2));
    return (((_e1 * 0.56) + (_e10 * 0.26)) + (_e20 * 0.12)) + (_e30 * 0.06);
}

metal::float2 cloud_warp(
    metal::float2 q,
    constant Cloud& cloud
) {
    metal::float2 d = cloud.drift;
    float _e7 = cloud_noise((q * 0.7) + d);
    float _e17 = cloud_noise(((q * 0.7) + metal::float2(8.3, 2.7)) - (d * 0.5));
    metal::float2 w_1 = metal::float2(_e7, _e17);
    return (w_1 - metal::float2(0.5)) * 0.7;
}

float cloud_density(
    metal::float2 q_1,
    metal::float2 warp,
    constant Cloud& cloud
) {
    metal::float2 _e5 = cloud.drift;
    float _e7 = billows((q_1 + warp) + _e5);
    float _e10 = cloud.cloud_cover;
    float low = 0.6 - (0.36 * _e10);
    return metal::smoothstep(low, low + 0.34, _e7);
}

Facet facet_at(
    metal::float2 r
) {
    float best = 1000000000.0;
    float second = 1000000000.0;
    Facet out = {};
    int j = -1;
    int i = {};
    metal::int2 cell_3 = naga_f2i32(metal::floor(r));
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            int _e44 = j;
            j = as_type<int>(as_type<uint>(_e44) + as_type<uint>(1));
        }
        loop_init = false;
        int _e10 = j;
        if (_e10 <= 1) {
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
                    int _e41 = i;
                    i = as_type<int>(as_type<uint>(_e41) + as_type<uint>(1));
                }
                loop_init_1 = false;
                int _e15 = i;
                if (_e15 <= 1) {
                } else {
                    break;
                }
                {
                    int _e18 = i;
                    int _e19 = j;
                    metal::int2 c = as_type<metal::int2>(as_type<metal::uint2>(cell_3) + as_type<metal::uint2>(metal::int2(_e18, _e19)));
                    metal::float2 _e26 = cloud_hash2_(c);
                    metal::float2 point = (static_cast<metal::float2>(c) + metal::float2(0.5)) + ((_e26 - metal::float2(0.5)) * 0.8);
                    float d_1 = metal::distance(r, point);
                    float _e34 = best;
                    if (d_1 < _e34) {
                        float _e36 = best;
                        second = _e36;
                        best = d_1;
                        out.centre = point;
                        out.id = c;
                    } else {
                        float _e39 = second;
                        if (d_1 < _e39) {
                            second = d_1;
                        }
                    }
                }
            }
        }
    }
    float _e48 = second;
    float _e49 = best;
    out.seam = _e48 - _e49;
    Facet _e51 = out;
    return _e51;
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
    metal::float2 q_2 = ((pt_1 - (_e35 * 0.5)) / metal::float2(_e42)) * units;
    metal::float2 _e46 = cloud_warp(q_2, cloud);
    float _e47 = cloud_density(q_2, _e46, cloud);
    if (_e47 <= 0.002) {
        return base;
    }
    float _e52 = cloud.scale_size;
    float scale_units = 6.0 / _e52;
    float _e58 = cloud.size.y;
    float scale_points = (_e58 / units) / scale_units;
    metal::float2 reach = metal::float2(scale_points * 0.75, 0.0);
    float _e67 = cloud_light(pt_1 + reach.xy, close_light, wide_light, cloud_sampler, cloud);
    float _e70 = cloud_light(pt_1 - reach.xy, close_light, wide_light, cloud_sampler, cloud);
    float _e74 = cloud_light(pt_1 + reach.yx, close_light, wide_light, cloud_sampler, cloud);
    float _e77 = cloud_light(pt_1 - reach.yx, close_light, wide_light, cloud_sampler, cloud);
    metal::float2 grad = metal::float2(_e67 - _e70, _e74 - _e77);
    float magnitude = metal::length(grad);
    float directed = metal::smoothstep(0.0, 0.03, magnitude);
    metal::float2 toward = metal::normalize(metal::mix(metal::float2(0.55, -0.83), grad / metal::float2(metal::max(magnitude, 0.00001)), directed));
    float aimed = 0.5 + (0.5 * directed);
    metal::float2 _e99 = cloud.drift;
    metal::float2 r_1 = (q_2 + _e99) * scale_units;
    Facet _e102 = facet_at(r_1);
    metal::float2 _e108 = cloud.drift;
    float _e115 = cloud.size.y;
    metal::float2 _e119 = cloud.size;
    metal::float2 centre_pt = ((((_e102.centre / metal::float2(scale_units)) - _e108) / metal::float2(units)) * _e115) + (_e119 * 0.5);
    float inside = metal::smoothstep(0.0, 0.3, _e102.seam);
    float _e127 = cloud_light(pt_1, close_light, wide_light, cloud_sampler, cloud);
    float _e128 = cloud_light(centre_pt, close_light, wide_light, cloud_sampler, cloud);
    float light = metal::mix(_e127, _e128, 0.6 * inside);
    metal::float2 _e133 = cloud_hash2_(_e102.id);
    float _e136 = cloud.time;
    float _e150 = cloud.time;
    metal::float2 rock = 0.12 * metal::float2(metal::sin((_e136 * (0.2 + (0.3 * _e133.x))) + (_e133.y * 6.2832)), metal::cos((_e150 * (0.25 + (0.2 * _e133.y))) + (_e133.x * 6.2832)));
    float _e167 = cloud.scale_glint;
    float relief = (_e167 * _e47) * inside;
    metal::float3 normal = metal::normalize(metal::float3((-(((r_1 - _e102.centre) + rock)) * 1.2) * relief, 1.0));
    metal::float3 sun = metal::normalize(metal::float3(toward * 0.7, 0.7));
    float diffuse = metal::max(metal::dot(normal, sun), 0.0) / sun.z;
    metal::float3 half_ = metal::normalize(sun + metal::float3(0.0, 0.0, 1.0));
    float flat_glint = metal::pow(half_.z, 24.0);
    float glint = (metal::max(metal::pow(metal::max(metal::dot(normal, half_), 0.0), 24.0) - flat_glint, 0.0) / (1.0 - flat_glint)) * aimed;
    float _e214 = cloud_density(q_2 + (toward * 0.18), _e46, cloud);
    float rim = metal::clamp((_e47 - _e214) * 2.5, 0.0, 1.0) * aimed;
    float shade = 1.2 - (0.4 * _e47);
    float _e235 = cloud.cloud_ambient;
    float lit = (((light * diffuse) * shade) * (0.8 + (0.6 * rim))) + _e235;
    metal::float3 _e240 = palette_color(metal::clamp(lit, 0.0, 1.0), lut);
    float _e243 = cloud.scale_glint;
    metal::float3 body = _e240 + metal::float3(((glint * _e243) * light) * 0.5);
    float _e252 = cloud.cloud_depth;
    return metal::mix(base, body, _e252 * _e47);
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

struct fs_cloud_backdrop_gammaInput {
    float slab [[user(loc0), center_perspective]];
    float t [[user(loc1), center_perspective]];
};
struct fs_cloud_backdrop_gammaOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_cloud_backdrop_gammaOutput fs_cloud_backdrop_gamma(
  fs_cloud_backdrop_gammaInput varyings [[stage_in]]
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
    return fs_cloud_backdrop_gammaOutput { _e8 };
}
