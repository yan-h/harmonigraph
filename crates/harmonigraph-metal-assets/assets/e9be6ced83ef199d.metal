// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct Pile {
    metal::float2 face;
    metal::float2 to_centre;
};
struct Geometry {
    float fuzz;
    float lobe;
    float variety;
    float padding;
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
constant float WASH_FBM_FINE = 2.07;
constant float WASH_FBM_FINE_TILED = 2.0;
constant float DOME_RADIUS = 1.15;
constant float DOME_JITTER = 0.3;
constant float DOME_RADIUS_MIN = 0.95;
constant float DOME_RADIUS_MAX = 1.32;
constant float DOME_UNION = 9.0;
constant float DOME_VARIETY_GAIN = 5.0;
constant float DOME_FACE = 1.5122874;
constant float DOME_LACUNARITY = 2.1;
constant float DOME_FINE_GAIN = 0.22;
constant float PERIOD = 40.0;

metal::int2 naga_mod(metal::int2 lhs, metal::int2 rhs) {
    metal::int2 divisor = metal::select(rhs, 1, (lhs == (-2147483647 - 1) & rhs == -1) | (rhs == 0));
    return lhs - (lhs / divisor) * divisor;
}

metal::int2 wrap_cell_for_tile(
    metal::int2 cell,
    int period
) {
    if (period <= 0) {
        return cell;
    }
    return naga_mod(as_type<metal::int2>(as_type<metal::uint2>(naga_mod(cell, metal::int2(period))) + as_type<metal::uint2>(metal::int2(period))), metal::int2(period));
}

metal::int2 wrap_cell(
    metal::int2 cell_1,
    int period_1
) {
    metal::int2 _e2 = wrap_cell_for_tile(cell_1, period_1);
    return _e2;
}

metal::float4 cloud_hash4_(
    metal::int2 cell_2
) {
    uint n = {};
    uint m = {};
    n = (as_type<uint>(cell_2.x) * 2654435769u) ^ (as_type<uint>(cell_2.y) * 2246822507u);
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
    int period_2,
    float variety
) {
    float weight = 0.0;
    metal::float2 face = metal::float2(0.0);
    metal::float2 to_centre = metal::float2(0.0);
    int j = -1;
    int i = {};
    float gain = {};
    Pile out = {};
    metal::float2 base = metal::floor(r);
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            int _e98 = j;
            j = as_type<int>(as_type<uint>(_e98) + as_type<uint>(1));
        }
        loop_init = false;
        int _e14 = j;
        if (_e14 <= 1) {
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
                    int _e95 = i;
                    i = as_type<int>(as_type<uint>(_e95) + as_type<uint>(1));
                }
                loop_init_1 = false;
                int _e19 = i;
                if (_e19 <= 1) {
                } else {
                    break;
                }
                {
                    int _e23 = i;
                    int _e24 = j;
                    metal::int2 cell_3 = as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(base)) + as_type<metal::uint2>(metal::int2(_e23, _e24)));
                    metal::int2 _e27 = wrap_cell(cell_3, period_2);
                    metal::float4 _e28 = cloud_hash4_(_e27);
                    int _e29 = i;
                    int _e31 = j;
                    metal::float2 centre = ((base + metal::float2(static_cast<float>(_e29), static_cast<float>(_e31))) + metal::float2(0.5)) + ((_e28.xy - metal::float2(0.5)) * DOME_JITTER);
                    float radius = metal::mix(DOME_RADIUS, metal::mix(DOME_RADIUS_MIN, DOME_RADIUS_MAX, _e28.z), variety);
                    metal::float2 d = (r - centre) / metal::float2(radius);
                    float q = 1.0 - metal::dot(d, d);
                    if (q <= 0.0) {
                        continue;
                    }
                    float root = metal::sqrt(q);
                    float h = q * root;
                    gain = 1.0;
                    if (variety > 0.0) {
                        gain = metal::exp2((DOME_VARIETY_GAIN * variety) * ((2.0 * _e28.w) - 1.0));
                    }
                    float _e74 = gain;
                    float w = _e74 * (metal::exp(DOME_UNION * h) - 1.0);
                    float _e81 = weight;
                    weight = _e81 + w;
                    metal::float2 _e83 = face;
                    face = _e83 + ((w * -(((DOME_FACE * root) * radius))) * d);
                    metal::float2 _e91 = to_centre;
                    to_centre = _e91 + (w * (centre - r));
                }
            }
        }
    }
    float _e102 = weight;
    if (_e102 <= 0.0) {
        out.face = metal::float2(0.0);
        out.to_centre = metal::float2(0.0);
        Pile _e111 = out;
        return _e111;
    }
    metal::float2 _e113 = face;
    float _e114 = weight;
    out.face = _e113 / metal::float2(_e114);
    metal::float2 _e118 = to_centre;
    float _e119 = weight;
    out.to_centre = _e118 / metal::float2(_e119);
    Pile _e122 = out;
    return _e122;
}

int naga_f2i32(float value) {
    return static_cast<int>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

Pile mosaic_field(
    metal::float2 r_1,
    int period_3,
    float variety_1
) {
    Pile out_1 = {};
    Pile _e3 = dome_octave(r_1, period_3, variety_1);
    Pile _e15 = dome_octave((r_1 * DOME_LACUNARITY) + metal::float2(17.3, 5.9), naga_f2i32(metal::rint(DOME_LACUNARITY * static_cast<float>(period_3))), variety_1);
    out_1.face = (_e3.face + (0.46199998 * _e15.face)) / metal::float2(1.22);
    out_1.to_centre = _e3.to_centre;
    Pile _e28 = out_1;
    return _e28;
}

struct fs_mosaic_tileInput {
    metal::float2 uv [[user(loc0), center_perspective]];
};
struct fs_mosaic_tileOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_mosaic_tileOutput fs_mosaic_tile(
  fs_mosaic_tileInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Geometry& geometry [[buffer(0)]]
) {
    const Vertex in = { position, varyings.uv };
    float _e7 = geometry.variety;
    Pile _e8 = mosaic_field(in.uv * PERIOD, 40, _e7);
    return fs_mosaic_tileOutput { metal::float4(_e8.face, _e8.to_centre) };
}
