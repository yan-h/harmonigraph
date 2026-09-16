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
    float scale_overlap;
    float scale_glint;
    float cloud_ambient;
    float _pad2_;
    float _pad3_;
};
struct Puff {
    bool hit;
    char _pad1[3];
    float cap;
    metal::float2 offset;
    float radius;
    float lift;
};
struct Pile {
    float density;
    char _pad1[12];
    metal::float3 normal;
};
struct Backlight {
    float glow;
    char _pad1[4];
    metal::float2 toward;
    float aimed;
    char _pad3[4];
};
constant float PUFF_JITTER = 0.7;
constant float PUFF_RADIUS_MAX = 1.1;
constant int PUFF_OCTAVES = 3;
constant float PUFF_LIFT = 0.35;
constant float PUFF_UNION = 8.0;
constant uint PUFF_SALT_B = 3266489909u;
constant uint PUFF_SALT_C = 668265263u;

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

uint cloud_bits(
    metal::int2 cell,
    uint salt
) {
    uint n = {};
    n = ((as_type<uint>(cell.x) * 2654435769u) ^ as_type<uint>(cell.y)) + salt;
    uint _e11 = n;
    uint _e12 = n;
    n = (_e11 ^ (_e12 >> 16u)) * 2146121005u;
    uint _e18 = n;
    uint _e19 = n;
    n = (_e18 ^ (_e19 >> 15u)) * 2221713035u;
    uint _e25 = n;
    uint _e26 = n;
    return _e25 ^ (_e26 >> 16u);
}

float cloud_slice(
    uint bits,
    uint shift
) {
    return static_cast<float>((bits >> shift) & 1023u) / 1023.0;
}

float puff_wave(
    float phase,
    float rate,
    constant Cloud& cloud
) {
    float turns = 5.0 + metal::floor(rate * 36.0);
    float _e9 = cloud.time;
    float t = metal::fract(((_e9 * turns) * 0.001) + phase);
    float ramp = metal::abs((t * 2.0) - 1.0);
    return (((ramp * ramp) * (3.0 - (2.0 * ramp))) * 2.0) - 1.0;
}

float cloud_occupancy(
    int octave,
    constant Cloud& cloud
) {
    float _e3 = cloud.cloud_cover;
    return metal::clamp((0.12 + (0.62 * _e3)) + (0.1 * static_cast<float>(octave)), 0.0, 1.0);
}

float cloud_octave_weight(
    int octave_1
) {
    return metal::pow(0.5, static_cast<float>(octave_1));
}

Puff cloud_puff(
    metal::int2 cell_1,
    metal::float2 at,
    uint salt_1,
    float occupancy,
    float radius_lo,
    float radius_hi,
    bool wander,
    constant Cloud& cloud
) {
    Puff out = Puff {false, {}, 0.0, metal::float2(0.0), 1.0, 0.0};
    metal::float2 stray = {};
    uint _e15 = cloud_bits(cell_1, salt_1);
    uint _e18 = cloud_bits(cell_1, salt_1 ^ PUFF_SALT_B);
    float _e20 = cloud_slice(_e18, 20u);
    if (_e20 >= occupancy) {
        Puff _e22 = out;
        return _e22;
    }
    float _e24 = cloud_slice(_e15, 0u);
    float _e26 = cloud_slice(_e15, 10u);
    stray = metal::float2(_e24, _e26) - metal::float2(0.5);
    if (wander) {
        uint _e34 = cloud_bits(cell_1, salt_1 ^ PUFF_SALT_C);
        float _e36 = cloud_slice(_e34, 20u);
        metal::float2 _e37 = stray;
        float _e39 = cloud_slice(_e34, 0u);
        float _e40 = puff_wave(_e39, _e36, cloud);
        float _e42 = cloud_slice(_e34, 10u);
        float _e43 = puff_wave(_e42, _e36, cloud);
        stray = _e37 + (0.18 * metal::float2(_e40, _e43));
    }
    metal::float2 _e52 = stray;
    metal::float2 centre = (static_cast<metal::float2>(cell_1) + metal::float2(0.5)) + (_e52 * PUFF_JITTER);
    metal::float2 x_2 = at - centre;
    float _e58 = cloud_slice(_e15, 20u);
    float radius = metal::mix(radius_lo, radius_hi, _e58);
    float u = 1.0 - (metal::dot(x_2, x_2) / (radius * radius));
    if (u <= 0.0) {
        Puff _e67 = out;
        return _e67;
    }
    out.hit = true;
    out.cap = metal::sqrt(u);
    out.offset = x_2 / metal::float2(radius);
    out.radius = radius;
    float _e78 = cloud_slice(_e18, 0u);
    out.lift = _e78;
    Puff _e79 = out;
    return _e79;
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

float cloud_mass(
    metal::float2 p,
    float radius_lo_1,
    float radius_hi_1,
    constant Cloud& cloud
) {
    float total = 0.0;
    int j = -1;
    int i = {};
    metal::int2 home = naga_f2i32(metal::floor(p));
    float _e6 = cloud_occupancy(0, cloud);
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            int _e37 = j;
            j = as_type<int>(as_type<uint>(_e37) + as_type<uint>(1));
        }
        loop_init = false;
        int _e11 = j;
        if (_e11 <= 1) {
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
                    int _e34 = i;
                    i = as_type<int>(as_type<uint>(_e34) + as_type<uint>(1));
                }
                loop_init_1 = false;
                int _e16 = i;
                if (_e16 <= 1) {
                } else {
                    break;
                }
                {
                    int _e19 = i;
                    int _e20 = j;
                    Puff _e25 = cloud_puff(as_type<metal::int2>(as_type<metal::uint2>(home) + as_type<metal::uint2>(metal::int2(_e19, _e20))), p, 0u, _e6, radius_lo_1, radius_hi_1, true, cloud);
                    if (_e25.hit) {
                        float _e27 = total;
                        total = _e27 + ((_e25.cap * _e25.cap) * _e25.cap);
                    }
                }
            }
        }
    }
    float _e40 = total;
    return _e40;
}

Pile cloud_pile(
    metal::float2 p_1,
    float lacunarity,
    float radius_lo_2,
    float radius_hi_2,
    constant Cloud& cloud
) {
    float union_acc = 0.0;
    metal::float3 normal_acc = metal::float3(0.0);
    float density = 0.0;
    float freq = 1.0;
    int octave_2 = 0;
    int j_1 = {};
    int i_1 = {};
    Pile out_1 = {};
    uint2 loop_bound_2 = uint2(4294967295u);
    bool loop_init_2 = true;
    while(true) {
        if (metal::all(loop_bound_2 == uint2(0u))) { break; }
        loop_bound_2 -= uint2(loop_bound_2.y == 0u, 1u);
        if (!loop_init_2) {
            int _e102 = octave_2;
            octave_2 = as_type<int>(as_type<uint>(_e102) + as_type<uint>(1));
        }
        loop_init_2 = false;
        int _e15 = octave_2;
        if (_e15 < PUFF_OCTAVES) {
        } else {
            break;
        }
        {
            int _e18 = octave_2;
            int _e22 = octave_2;
            metal::float2 shift_1 = metal::float2(static_cast<float>(_e18) * 31.7, static_cast<float>(_e22) * -17.3);
            float _e27 = freq;
            metal::float2 at_1 = (p_1 * _e27) + shift_1;
            metal::int2 home_1 = naga_f2i32(metal::floor(at_1));
            int _e32 = octave_2;
            uint salt_2 = static_cast<uint>(_e32) * 2654435769u;
            int _e36 = octave_2;
            float _e37 = cloud_occupancy(_e36, cloud);
            int _e38 = octave_2;
            float _e39 = cloud_octave_weight(_e38);
            j_1 = -1;
            uint2 loop_bound_3 = uint2(4294967295u);
            bool loop_init_3 = true;
            while(true) {
                if (metal::all(loop_bound_3 == uint2(0u))) { break; }
                loop_bound_3 -= uint2(loop_bound_3.y == 0u, 1u);
                if (!loop_init_3) {
                    int _e97 = j_1;
                    j_1 = as_type<int>(as_type<uint>(_e97) + as_type<uint>(1));
                }
                loop_init_3 = false;
                int _e42 = j_1;
                if (_e42 <= 1) {
                } else {
                    break;
                }
                {
                    i_1 = -1;
                    uint2 loop_bound_4 = uint2(4294967295u);
                    bool loop_init_4 = true;
                    while(true) {
                        if (metal::all(loop_bound_4 == uint2(0u))) { break; }
                        loop_bound_4 -= uint2(loop_bound_4.y == 0u, 1u);
                        if (!loop_init_4) {
                            int _e94 = i_1;
                            i_1 = as_type<int>(as_type<uint>(_e94) + as_type<uint>(1));
                        }
                        loop_init_4 = false;
                        int _e47 = i_1;
                        if (_e47 <= 1) {
                        } else {
                            break;
                        }
                        {
                            int _e50 = i_1;
                            int _e51 = j_1;
                            int _e54 = octave_2;
                            Puff _e57 = cloud_puff(as_type<metal::int2>(as_type<metal::uint2>(home_1) + as_type<metal::uint2>(metal::int2(_e50, _e51))), at_1, salt_2, _e37, radius_lo_2, radius_hi_2, _e54 == 0, cloud);
                            if (!(_e57.hit)) {
                                continue;
                            }
                            float _e60 = density;
                            density = _e60 + (((_e57.cap * _e57.cap) * _e57.cap) * _e39);
                            float _e75 = freq;
                            float height = ((_e57.radius * _e57.cap) + (PUFF_LIFT * _e57.lift)) / _e75;
                            float share = metal::exp(PUFF_UNION * height) - 1.0;
                            float _e82 = union_acc;
                            union_acc = _e82 + share;
                            metal::float3 _e84 = normal_acc;
                            float _e86 = freq;
                            normal_acc = _e84 + (share * metal::float3(_e57.offset * _e86, metal::max(_e57.cap, 0.05)));
                        }
                    }
                }
            }
            float _e100 = freq;
            freq = _e100 * lacunarity;
        }
    }
    float _e107 = density;
    out_1.density = _e107;
    metal::float3 _e109 = normal_acc;
    out_1.normal = metal::normalize(_e109 + metal::float3(0.0, 0.0, 0.0001));
    Pile _e116 = out_1;
    return _e116;
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
    return (1.3 * _e10) + (0.25 * material_1);
}

metal::float3 softened(
    metal::float3 colour
) {
    float m = metal::max(metal::max(colour.x, colour.y), colour.z);
    if (m <= 0.75) {
        return colour;
    }
    return colour * ((0.75 + (0.25 * (1.0 - metal::exp((0.75 - m) * 4.0)))) / m);
}

Backlight backlight(
    metal::float2 pt_1,
    float reach,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    Backlight out_2 = {};
    metal::float2 d0_ = metal::float2(0.2887, 0.0);
    metal::float2 d1_ = metal::float2(-0.3687, 0.3377);
    metal::float2 d2_ = metal::float2(0.0564, -0.643);
    metal::float2 d3_ = metal::float2(0.4647, 0.6061);
    metal::float2 d4_ = metal::float2(-0.8528, -0.1508);
    metal::float2 d5_ = metal::float2(0.8078, -0.5139);
    float _e22 = cloud_light(pt_1 + (d0_ * reach), close_light, wide_light, cloud_sampler, cloud);
    float _e25 = cloud_light(pt_1 + (d1_ * reach), close_light, wide_light, cloud_sampler, cloud);
    float _e28 = cloud_light(pt_1 + (d2_ * reach), close_light, wide_light, cloud_sampler, cloud);
    float _e31 = cloud_light(pt_1 + (d3_ * reach), close_light, wide_light, cloud_sampler, cloud);
    float _e34 = cloud_light(pt_1 + (d4_ * reach), close_light, wide_light, cloud_sampler, cloud);
    float _e37 = cloud_light(pt_1 + (d5_ * reach), close_light, wide_light, cloud_sampler, cloud);
    float ring = ((((_e22 + _e25) + _e28) + _e31) + _e34) + _e37;
    float _e43 = cloud_light(pt_1, close_light, wide_light, cloud_sampler, cloud);
    out_2.glow = (_e43 * 0.2) + (ring * 0.1333);
    metal::float2 pull = (((((metal::normalize(d0_) * _e22) + (metal::normalize(d1_) * _e25)) + (metal::normalize(d2_) * _e28)) + (metal::normalize(d3_) * _e31)) + (metal::normalize(d4_) * _e34)) + (metal::normalize(d5_) * _e37);
    float lean = metal::length(pull) / metal::max(ring, 0.0001);
    float followed = 0.25 * metal::smoothstep(0.02, 0.25, lean);
    out_2.toward = metal::normalize(metal::mix(metal::float2(0.55, -0.83), pull / metal::float2(metal::max(metal::length(pull), 0.00001)), followed));
    out_2.aimed = 0.45 + (0.55 * metal::smoothstep(0.0, 0.2, lean));
    Backlight _e96 = out_2;
    return _e96;
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
    float walked = 0.0;
    float travelled = 0.0;
    int step = 0;
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
    metal::float2 pt_2 = (position_1 / metal::float2(_e21)) - _e26;
    float _e30 = cloud.cloud_scale;
    float units = 5.0 / _e30;
    metal::float2 _e35 = cloud.size;
    float _e42 = cloud.size.y;
    metal::float2 _e48 = cloud.drift;
    metal::float2 q = (((pt_2 - (_e35 * 0.5)) / metal::float2(_e42)) * units) + _e48;
    float _e52 = cloud.scale_size;
    float lacunarity_1 = metal::max(1.2, metal::sqrt(4.0 / _e52));
    float _e60 = cloud.scale_overlap;
    float radius_hi_3 = metal::min(_e60, PUFF_RADIUS_MAX);
    float radius_lo_3 = radius_hi_3 * 0.48;
    Pile _e65 = cloud_pile(q, lacunarity_1, radius_lo_3, radius_hi_3, cloud);
    float alpha = 1.0 - metal::exp(-2.4 * metal::pow(metal::max(_e65.density, 0.0), 1.15));
    if (alpha <= 0.002) {
        return base;
    }
    float _e81 = cloud.size.y;
    float cloud_points = _e81 / units;
    float _e88 = cloud.size.y;
    Backlight _e92 = backlight(pt_2, metal::max(cloud_points * 2.0, _e88 * 0.2), close_light, wide_light, cloud_sampler, cloud);
    uint2 loop_bound_5 = uint2(4294967295u);
    bool loop_init_5 = true;
    while(true) {
        if (metal::all(loop_bound_5 == uint2(0u))) { break; }
        loop_bound_5 -= uint2(loop_bound_5.y == 0u, 1u);
        if (!loop_init_5) {
            int _e124 = step;
            step = as_type<int>(as_type<uint>(_e124) + as_type<uint>(1));
        }
        loop_init_5 = false;
        int _e99 = step;
        if (_e99 < 2) {
        } else {
            break;
        }
        {
            float _e102 = travelled;
            int _e103 = step;
            travelled = _e102 + (0.95 * (1.0 + (static_cast<float>(_e103) * 0.8)));
            float _e112 = walked;
            float _e114 = travelled;
            float _e117 = cloud_mass(q + (_e92.toward * _e114), radius_lo_3, radius_hi_3, cloud);
            int _e118 = step;
            walked = _e112 + (_e117 / (1.0 + static_cast<float>(_e118)));
        }
    }
    float _e127 = walked;
    float shadow = metal::exp(-1.35 * _e127);
    metal::float3 sun = metal::normalize(metal::float3(_e92.toward * 0.97, -0.25));
    float key = metal::pow(metal::max((metal::dot(_e65.normal, sun) * 0.5) + 0.5, 0.0), 2.0);
    float through = metal::exp(-0.9 * _e65.density);
    float _e153 = cloud.scale_glint;
    float sculpt = 2.6 * _e153;
    float _e158 = cloud.cloud_ambient;
    float shading = ((_e158 + ((key * shadow) * sculpt)) + (((0.95 * through) * shadow) * _e92.aimed)) * 1.3;
    float tint = metal::pow(metal::clamp(_e92.glow, 0.0, 1.0), 0.7);
    float level_3 = metal::clamp((tint * 0.85) + 0.15, 0.0, 1.0);
    metal::float3 _e183 = palette_color(level_3, lut);
    metal::float3 body = _e183 * shading;
    metal::float3 _e185 = softened(body);
    float _e188 = cloud.cloud_depth;
    return metal::mix(base, _e185, _e188 * alpha);
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
