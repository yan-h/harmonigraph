// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

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
    char _pad6[4];
};
struct Wet {
    metal::float2 offset;
};
struct WashField {
    Wet coarse;
    Wet fine;
    float cover;
    char _pad3[4];
};
struct Vertex {
    metal::float4 position;
    metal::float2 uv;
    char _pad2[8];
};
struct TileOut {
    metal::float4 a;
    metal::float4 b;
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

metal::float3 wash_hash(
    metal::int2 cell_2,
    uint salt
) {
    uint n = {};
    n = (as_type<uint>(cell_2.x) * 2654435769u) ^ (as_type<uint>(cell_2.y) * 2246822507u);
    uint _e12 = n;
    n = _e12 ^ (salt * 668265261u);
    uint _e16 = n;
    uint _e17 = n;
    n = (_e16 ^ (_e17 >> 16u)) * 2146121005u;
    uint _e23 = n;
    uint _e24 = n;
    n = (_e23 ^ (_e24 >> 15u)) * 2221713035u;
    uint _e30 = n;
    uint _e31 = n;
    n = _e30 ^ (_e31 >> 16u);
    uint _e35 = n;
    uint _e41 = n;
    uint _e49 = n;
    return metal::float3(static_cast<float>(_e35 & 1023u) / 1023.0, static_cast<float>((_e41 >> 10u) & 1023u) / 1023.0, static_cast<float>((_e49 >> 20u) & 1023u) / 1023.0);
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

float wash_noise(
    metal::float2 p,
    uint salt_1,
    int period_2
) {
    metal::float2 b = metal::floor(p);
    metal::float2 f_1 = p - b;
    metal::float2 t = (f_1 * f_1) * (metal::float2(3.0) - (2.0 * f_1));
    metal::int2 i_1 = naga_f2i32(b);
    metal::int2 _e13 = wrap_cell(i_1, period_2);
    metal::float3 _e14 = wash_hash(_e13, salt_1);
    float n00_ = _e14.x;
    metal::int2 _e20 = wrap_cell(as_type<metal::int2>(as_type<metal::uint2>(i_1) + as_type<metal::uint2>(metal::int2(1, 0))), period_2);
    metal::float3 _e21 = wash_hash(_e20, salt_1);
    float n10_ = _e21.x;
    metal::int2 _e27 = wrap_cell(as_type<metal::int2>(as_type<metal::uint2>(i_1) + as_type<metal::uint2>(metal::int2(0, 1))), period_2);
    metal::float3 _e28 = wash_hash(_e27, salt_1);
    float n01_ = _e28.x;
    metal::int2 _e34 = wrap_cell(as_type<metal::int2>(as_type<metal::uint2>(i_1) + as_type<metal::uint2>(metal::int2(1, 1))), period_2);
    metal::float3 _e35 = wash_hash(_e34, salt_1);
    float n11_ = _e35.x;
    return metal::mix(metal::mix(n00_, n10_, t.x), metal::mix(n01_, n11_, t.x), t.y);
}

float wash_fbm(
    metal::float2 p_1,
    uint salt_2,
    int period_3
) {
    float lacunarity = (period_3 > 0) ? WASH_FBM_FINE_TILED : WASH_FBM_FINE;
    float _e8 = wash_noise(p_1, salt_2, period_3);
    float _e18 = wash_noise((p_1 * lacunarity) + metal::float2(13.1, -7.3), salt_2 + 31u, as_type<int>(as_type<uint>(period_3) * as_type<uint>(2)));
    return (_e8 + (0.5 * _e18)) / 1.5;
}

Glob wash_glob(
    metal::int2 cell_3,
    uint salt_3,
    metal::float2 r,
    float occupancy,
    int period_4
) {
    Glob out = {};
    metal::int2 _e5 = wrap_cell(cell_3, period_4);
    metal::float3 _e8 = wash_hash(_e5, salt_3 + 77u);
    out.order = _e8.x;
    if (_e8.y > occupancy) {
        out.centre = r;
        out.edge = 1000000000.0;
        Glob _e17 = out;
        return _e17;
    }
    metal::float3 _e18 = wash_hash(_e5, salt_3);
    metal::float2 centre = (static_cast<metal::float2>(cell_3) + metal::float2(0.5)) + ((_e18.xy - metal::float2(0.5)) * WASH_JITTER);
    float radius = metal::mix(WASH_RADIUS_MIN, WASH_RADIUS_MAX, _e18.z);
    out.centre = centre;
    out.edge = metal::length(r - centre) / radius;
    Glob _e39 = out;
    return _e39;
}

Wash wash_scan(
    metal::float2 r_1,
    uint salt_4,
    float occupancy_1,
    int period_5
) {
    Wash out_1 = {};
    float best = -1000000000.0;
    float second = -1000000000.0;
    float near_a = -1000000000.0;
    float near_b = -1000000000.0;
    float order_a = -1000000000.0;
    float order_b = -1000000000.0;
    metal::float2 front_a = {};
    metal::float2 front_b = {};
    int j = -2;
    int i = {};
    out_1.centre = r_1;
    out_1.under = r_1;
    out_1.front = r_1;
    out_1.near = -1000000000.0;
    out_1.edge = 1.0;
    out_1.cover = 0.0;
    front_a = r_1;
    front_b = r_1;
    metal::int2 base = naga_f2i32(metal::floor(r_1));
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            int _e92 = j;
            j = as_type<int>(as_type<uint>(_e92) + as_type<uint>(1));
        }
        loop_init = false;
        int _e32 = j;
        if (_e32 <= WASH_RING) {
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
                    int _e89 = i;
                    i = as_type<int>(as_type<uint>(_e89) + as_type<uint>(1));
                }
                loop_init_1 = false;
                int _e37 = i;
                if (_e37 <= WASH_RING) {
                } else {
                    break;
                }
                {
                    int _e40 = i;
                    int _e41 = j;
                    Glob _e44 = wash_glob(as_type<metal::int2>(as_type<metal::uint2>(base) + as_type<metal::uint2>(metal::int2(_e40, _e41))), salt_4, r_1, occupancy_1, period_5);
                    float prox = 1.0 - _e44.edge;
                    float _e50 = out_1.cover;
                    out_1.cover = metal::max(_e50, metal::clamp(prox / 0.05, 0.0, 1.0));
                    if (_e44.edge < 1.0) {
                        float _e61 = best;
                        if (_e44.order > _e61) {
                            float _e63 = best;
                            second = _e63;
                            metal::float2 _e66 = out_1.centre;
                            out_1.under = _e66;
                            best = _e44.order;
                            out_1.centre = _e44.centre;
                            out_1.edge = _e44.edge;
                        } else {
                            float _e73 = second;
                            if (_e44.order > _e73) {
                                second = _e44.order;
                                out_1.under = _e44.centre;
                            }
                        }
                    } else {
                        float _e78 = near_a;
                        if (prox > _e78) {
                            float _e80 = near_a;
                            near_b = _e80;
                            float _e81 = order_a;
                            order_b = _e81;
                            metal::float2 _e82 = front_a;
                            front_b = _e82;
                            near_a = prox;
                            order_a = _e44.order;
                            front_a = _e44.centre;
                        } else {
                            float _e85 = near_b;
                            if (prox > _e85) {
                                near_b = prox;
                                order_b = _e44.order;
                                front_b = _e44.centre;
                            }
                        }
                    }
                }
            }
        }
    }
    float _e95 = order_b;
    float _e96 = best;
    if (_e95 > _e96) {
        float _e99 = near_b;
        out_1.near = _e99;
        metal::float2 _e101 = front_b;
        out_1.front = _e101;
    }
    float _e102 = order_a;
    float _e103 = best;
    if (_e102 > _e103) {
        float _e106 = near_a;
        out_1.near = _e106;
        metal::float2 _e108 = front_a;
        out_1.front = _e108;
    }
    Wash _e109 = out_1;
    return _e109;
}

Wet wash_wet(
    Wash f,
    metal::float2 r_2,
    float fuzz
) {
    metal::float2 look = {};
    float fa = {};
    float bl = {};
    float feather = 0.1 + (0.8 * fuzz);
    float bleed = 0.12 + (0.78 * fuzz);
    look = f.centre;
    fa = metal::clamp((f.edge - (1.0 - feather)) / feather, 0.0, 1.0);
    float _e22 = fa;
    float _e23 = fa;
    float _e25 = fa;
    fa = ((_e22 * _e23) * (3.0 - (2.0 * _e25))) * 0.5;
    metal::float2 _e33 = look;
    float _e35 = fa;
    look = metal::mix(_e33, f.under, _e35);
    bl = metal::clamp((f.near + bleed) / bleed, 0.0, 1.0);
    float _e44 = bl;
    float _e45 = bl;
    float _e47 = bl;
    bl = ((_e44 * _e45) * (3.0 - (2.0 * _e47))) * 0.5;
    metal::float2 _e55 = look;
    float _e57 = bl;
    look = metal::mix(_e55, f.front, _e57);
    metal::float2 _e59 = look;
    return Wet {_e59 - r_2};
}

int naga_f2i32(float value) {
    return static_cast<int>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

WashField wash_field(
    metal::float2 r_3,
    int period_6,
    float fuzz_1,
    float lobe
) {
    metal::float2 warped = {};
    WashField out_2 = {};
    warped = r_3;
    if (lobe > 0.0) {
        float amp = WASH_WARP * lobe;
        int warp_period = naga_f2i32(metal::rint(WASH_WARP_SCALE * static_cast<float>(period_6)));
        metal::float2 _e14 = warped;
        float _e20 = wash_fbm(r_3 * WASH_WARP_SCALE, 71u, warp_period);
        float _e30 = wash_fbm((r_3 * WASH_WARP_SCALE) + metal::float2(37.0, -19.0), 73u, warp_period);
        warped = _e14 + ((amp * 2.0) * metal::float2(_e20 - 0.5, _e30 - 0.5));
    }
    metal::float2 _e38 = warped;
    Wash _e41 = wash_scan(_e38, 1u, 1.0, period_6);
    metal::float2 _e42 = warped;
    Wet _e43 = wash_wet(_e41, _e42, fuzz_1);
    out_2.coarse = _e43;
    metal::float2 _e44 = warped;
    metal::float2 fine_r = (_e44 * WASH_LACUNARITY) + metal::float2(17.3, 5.9);
    Wash _e58 = wash_scan(fine_r, 2u, WASH_FINE_OCCUPANCY, naga_f2i32(metal::rint(WASH_LACUNARITY * static_cast<float>(period_6))));
    Wet _e60 = wash_wet(_e58, fine_r, fuzz_1);
    out_2.fine = _e60;
    out_2.cover = _e58.cover;
    WashField _e63 = out_2;
    return _e63;
}

struct fs_tileInput {
    metal::float2 uv [[user(loc0), center_perspective]];
};
struct fs_tileOutput {
    metal::float4 a [[color(0)]];
    metal::float4 b [[color(1)]];
};
fragment fs_tileOutput fs_tile(
  fs_tileInput varyings [[stage_in]]
, metal::float4 position [[position]]
) {
    const Vertex in = { position, varyings.uv };
    WashField _e7 = wash_field(in.uv * PERIOD, 40, 0.25, 0.7);
    const auto _tmp = TileOut {metal::float4(_e7.coarse.offset, 0.0, 0.0), metal::float4(_e7.fine.offset, 0.0, _e7.cover)};
    return fs_tileOutput { _tmp.a, _tmp.b };
}
