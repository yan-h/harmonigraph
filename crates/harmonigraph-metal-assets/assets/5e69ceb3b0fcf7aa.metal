// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct VelvetBody {
    metal::float2 center;
    float weight;
    char _pad2[4];
};
struct Settings {
    metal::float2 size;
    float cell;
    float depth;
    metal::float2 drift;
    float refract;
    float layers;
    float randomness;
    char _pad7[4];
    metal::float2 velvet_form;
    metal::float4 velvet;
};
struct Vertex {
    metal::float4 position;
    metal::float2 uv;
    char _pad2[8];
};
constant float CLOUD_TILE_ROT_SIN = 0.6;
constant float CLOUD_TILE_ROT_COS = 0.8;
constant int WASH_RING = 2;
constant float WASH_JITTER = 0.4;
constant float WASH_RADIUS_MIN = 1.17;
constant float WASH_RADIUS_MAX = 1.91;
constant float WASH_CELLS = 5.25;
constant float WASH_LACUNARITY = 2.1;
constant float WASH_FINE_OCCUPANCY = 0.2;
constant float WASH_WARP = 0.45;
constant float WASH_WARP_SCALE = 0.9;
constant float WASH_FBM_FINE = 2.0;
constant float PERIOD = 40.0;

float velvet_hash(
    metal::int2 cell,
    uint seed
) {
    uint h = {};
    h = ((as_type<uint>(cell.x) * 1597334677u) ^ (as_type<uint>(cell.y) * 3812015801u)) ^ (seed * 2798796415u);
    uint _e15 = h;
    uint _e16 = h;
    h = (_e15 ^ (_e16 >> 16u)) * 2246822519u;
    uint _e22 = h;
    uint _e23 = h;
    h = (_e22 ^ (_e23 >> 13u)) * 3266489917u;
    uint _e29 = h;
    uint _e30 = h;
    h = _e29 ^ (_e30 >> 16u);
    uint _e34 = h;
    return static_cast<float>(_e34 >> 8u) / 16777216.0;
}

metal::float2 velvet_warp(
    metal::float2 p,
    float irregularity
) {
    return metal::float2(0.43 * metal::sin((p.y * 0.61) + metal::sin(p.x * 0.24)), 0.4 * metal::sin((p.x * 0.53) + metal::sin(p.y * 0.31))) * (irregularity / 0.8);
}

float velvet_square_radius(
    metal::float2 v,
    float p_1
) {
    metal::float2 a = metal::abs(v);
    float m = metal::max(metal::max(a.x, a.y), 0.000001);
    return m * metal::pow(1.0 + metal::pow(metal::min(a.x, a.y) / m, p_1), 1.0 / p_1);
}

VelvetBody velvet_body(
    metal::float2 q,
    metal::int2 cell_1,
    metal::float4 dials,
    float tilt,
    float p_2
) {
    float r = {};
    float _e6 = velvet_hash(cell_1, 0u);
    float _e8 = velvet_hash(cell_1, 1u);
    metal::float2 center = (static_cast<metal::float2>(cell_1) + metal::float2(0.5)) + ((metal::float2(_e6, _e8) - metal::float2(0.5)) * dials.y);
    metal::float2 delta = q - center;
    if (metal::any(metal::abs(delta) >= metal::float2(1.9))) {
        return VelvetBody {metal::float2(0.0), 0.0};
    }
    float _e31 = velvet_hash(cell_1, 2u);
    float _e33 = velvet_hash(cell_1, 3u);
    float angle = (0.2 + ((_e31 - 0.5) * 0.85)) * tilt;
    float ca = metal::cos(angle);
    float sa = metal::sin(angle);
    float size_1 = 0.875 + ((0.9 * dials.w) * (_e33 - 0.5));
    metal::float2 rotated = metal::float2((delta.x * ca) + (delta.y * sa), (-(delta.x) * sa) + (delta.y * ca)) / metal::float2(size_1);
    float rx = rotated.x / metal::mix(1.0, metal::clamp(1.0 - (0.38 * rotated.y), 0.52, 1.3), dials.z);
    metal::float2 shaped = metal::float2(rx, rotated.y + (((dials.z * 0.17) * rx) * rx));
    r = metal::length(shaped);
    if (p_2 > 2.0) {
        float _e90 = velvet_square_radius(shaped, p_2);
        r = _e90;
    }
    float support = (1.0 - metal::smoothstep(1.5, 1.9, metal::abs(delta.x))) * (1.0 - metal::smoothstep(1.5, 1.9, metal::abs(delta.y)));
    float _e112 = r;
    float body = 1.0 - metal::smoothstep(0.85 - dials.x, 0.85 + dials.x, _e112);
    float _e116 = r;
    float _e119 = r;
    float weight_1 = ((body + (0.025 * metal::exp((-2.0 * _e116) * _e119))) * metal::exp(2.8 * (_e31 - 0.5))) * support;
    metal::float2 _e133 = velvet_warp(center, dials.y);
    return VelvetBody {center - _e133, weight_1};
}

metal::float4 velvet_light(
    metal::texture2d<float, metal::access::sample> source,
    metal::sampler source_sampler,
    metal::float2 uv,
    metal::float2 radius
) {
    metal::float4 _e5 = source.sample(source_sampler, uv, metal::level(0.0));
    metal::float4 _e13 = source.sample(source_sampler, uv + metal::float2(radius.x, 0.0), metal::level(0.0));
    metal::float4 _e19 = source.sample(source_sampler, uv - metal::float2(radius.x, 0.0), metal::level(0.0));
    metal::float4 _e26 = source.sample(source_sampler, uv + metal::float2(0.0, radius.y), metal::level(0.0));
    metal::float4 _e33 = source.sample(source_sampler, uv - metal::float2(0.0, radius.y), metal::level(0.0));
    return (0.4 * _e5) + (0.15 * (((_e13 + _e19) + _e26) + _e33));
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

metal::float4 velvet_material(
    metal::texture2d<float, metal::access::sample> source_1,
    metal::sampler source_sampler_1,
    metal::float2 pt,
    metal::float2 size,
    float cell_size,
    metal::float2 drift,
    metal::float4 dials_1,
    metal::float2 form
) {
    metal::float4 light = metal::float4(0.0);
    float weight = 0.0;
    int j = -2;
    int i = {};
    metal::float2 p_3 = (pt / metal::float2(cell_size)) + drift;
    metal::float2 _e12 = velvet_warp(p_3, dials_1.y);
    metal::float2 q_1 = p_3 + _e12;
    metal::int2 base = naga_f2i32(metal::floor(q_1));
    float exponent = 2.0 * metal::pow(6.0, form.x);
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            int _e65 = j;
            j = as_type<int>(as_type<uint>(_e65) + as_type<uint>(1));
        }
        loop_init = false;
        int _e28 = j;
        if (_e28 <= 2) {
        } else {
            break;
        }
        {
            i = -2;
            uint2 loop_bound_1 = uint2(4294967295u);
            bool loop_init_1 = true;
            while(true) {
                if (metal::all(loop_bound_1 == uint2(0u))) { break; }
                loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
                if (!loop_init_1) {
                    int _e62 = i;
                    i = as_type<int>(as_type<uint>(_e62) + as_type<uint>(1));
                }
                loop_init_1 = false;
                int _e33 = i;
                if (_e33 <= 2) {
                } else {
                    break;
                }
                {
                    int _e36 = i;
                    int _e37 = j;
                    VelvetBody _e41 = velvet_body(q_1, as_type<metal::int2>(as_type<metal::uint2>(base) + as_type<metal::uint2>(metal::int2(_e36, _e37))), dials_1, form.y, exponent);
                    if (_e41.weight > 0.0) {
                        metal::float2 uv_1 = ((_e41.center - drift) * cell_size) / size;
                        metal::float4 _e49 = light;
                        metal::float4 _e55 = velvet_light(source_1, source_sampler_1, uv_1, metal::float2(0.14 * cell_size) / size);
                        light = _e49 + (_e41.weight * _e55);
                        float _e58 = weight;
                        weight = _e58 + _e41.weight;
                    }
                }
            }
        }
    }
    metal::float4 _e67 = light;
    float _e68 = weight;
    return _e67 / metal::float4(metal::max(_e68, 0.00000000000000000001));
}

struct fs_velvetInput {
    metal::float2 uv [[user(loc0), center_perspective]];
};
struct fs_velvetOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_velvetOutput fs_velvet(
  fs_velvetInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Settings& settings [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> source_2 [[texture(0)]]
, metal::sampler source_sampler_2 [[sampler(0)]]
) {
    const Vertex in = { position, varyings.uv };
    metal::float2 _e6 = settings.size;
    metal::float2 _e10 = settings.size;
    float _e13 = settings.cell;
    metal::float2 _e16 = settings.drift;
    metal::float4 _e19 = settings.velvet;
    metal::float2 _e22 = settings.velvet_form;
    metal::float4 _e23 = velvet_material(source_2, source_sampler_2, in.uv * _e6, _e10, _e13, _e16, _e19, _e22);
    metal::float4 raw = source_2.sample(source_sampler_2, in.uv, metal::level(0.0));
    float _e31 = settings.depth;
    return fs_velvetOutput { metal::mix(raw, _e23, _e31) };
}
