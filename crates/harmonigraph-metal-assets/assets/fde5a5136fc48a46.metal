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
    float contour_strength;
    uint tone_baked;
    metal::float2 drift;
    float cloud_depth;
    float scale_size;
    float scale_variety;
    float scale_refract;
    float scale_relief;
    uint cloud_style;
    float wash_size;
    float wash_fuzz;
    float wash_ragged;
    float wash_lobe;
    float wash_refract;
    float wash_pool;
    float wash_layers;
    uint tile_cells;
};
struct Pile {
    metal::float2 face;
    metal::float2 to_centre;
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
    char _pad6[4];
};
struct Painted {
    float tone;
    float hold;
};
struct Wet {
    metal::float2 offset;
    float pigment;
    char _pad2[4];
};
struct WashField {
    Wet coarse;
    Wet fine;
    float cover;
    char _pad3[4];
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
constant float SUN_LEAN = 1.0;
constant float SUN_KNEE = 0.03;
constant float RELIEF_FLOOR_FALL = 3.22;
constant float DOME_LACUNARITY = 2.1;
constant float DOME_FINE_GAIN = 0.22;
constant float CLOUD_SHADE = 0.64;
constant int WASH_RING = 2;
constant float WASH_JITTER = 0.4;
constant float WASH_RAGGED = 0.3;
constant float WASH_RADIUS_MIN = 1.02;
constant float WASH_RADIUS_MAX = 1.66;
constant float WASH_CELLS = 5.25;
constant float WASH_LACUNARITY = 2.1;
constant float WASH_FINE_OCCUPANCY = 0.2;
constant float WASH_WARP = 0.45;
constant float WASH_WARP_SCALE = 0.9;
constant float WASH_RAGGED_SCALE = 2.8;
constant float WASH_POOL = 0.44;
constant float WASH_POOL_WIDTH = 0.55;
constant float WASH_SURF = 0.07;
constant float WASH_PIG_DEPTH = 0.35;
constant float WASH_TONE_FLOOR = 0.05;
constant float WASH_PIVOT = 0.45;
constant float WASH_LIFT_A = 1.15;
constant float WASH_LIFT_B = 0.16;
constant float WASH_BLACK_KNEE = 0.175;
constant float WASH_FBM_FINE = 2.07;
constant float WASH_FBM_FINE_TILED = 2.0;

float density_decode(
    float value
) {
    float y = metal::max(value, 0.0);
    return (2.0 * y) / (0.1 + metal::sqrt(0.01 + (3.6 * y)));
}

metal::int2 naga_mod(metal::int2 lhs, metal::int2 rhs) {
    metal::int2 divisor = metal::select(rhs, 1, (lhs == (-2147483647 - 1) & rhs == -1) | (rhs == 0));
    return lhs - (lhs / divisor) * divisor;
}

metal::int2 wrap_cell(
    metal::int2 cell,
    int period
) {
    if (period <= 0) {
        return cell;
    }
    metal::int2 p_2 = metal::int2(period);
    return naga_mod(as_type<metal::int2>(as_type<metal::uint2>(naga_mod(cell, p_2)) + as_type<metal::uint2>(p_2)), p_2);
}

metal::float4 cloud_hash4_(
    metal::int2 cell_1
) {
    uint n = {};
    uint m = {};
    n = (as_type<uint>(cell_1.x) * 2654435769u) ^ (as_type<uint>(cell_1.y) * 2246822507u);
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
    int period_1,
    constant Cloud& cloud
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
            int _e106 = j;
            j = as_type<int>(as_type<uint>(_e106) + as_type<uint>(1));
        }
        loop_init = false;
        int _e13 = j;
        if (_e13 <= 1) {
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
                    int _e103 = i;
                    i = as_type<int>(as_type<uint>(_e103) + as_type<uint>(1));
                }
                loop_init_1 = false;
                int _e18 = i;
                if (_e18 <= 1) {
                } else {
                    break;
                }
                {
                    int _e22 = i;
                    int _e23 = j;
                    metal::int2 cell_4 = as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(base)) + as_type<metal::uint2>(metal::int2(_e22, _e23)));
                    metal::int2 _e26 = wrap_cell(cell_4, period_1);
                    metal::float4 _e27 = cloud_hash4_(_e26);
                    int _e28 = i;
                    int _e30 = j;
                    metal::float2 centre = ((base + metal::float2(static_cast<float>(_e28), static_cast<float>(_e30))) + metal::float2(0.5)) + ((_e27.xy - metal::float2(0.5)) * DOME_JITTER);
                    float _e51 = cloud.scale_variety;
                    float radius = metal::mix(DOME_RADIUS, metal::mix(DOME_RADIUS_MIN, DOME_RADIUS_MAX, _e27.z), _e51);
                    metal::float2 d = (r - centre) / metal::float2(radius);
                    float q = 1.0 - metal::dot(d, d);
                    if (q <= 0.0) {
                        continue;
                    }
                    float root = metal::sqrt(q);
                    float h = q * root;
                    gain = 1.0;
                    float _e67 = cloud.scale_variety;
                    if (_e67 > 0.0) {
                        float _e73 = cloud.scale_variety;
                        gain = metal::exp2((DOME_VARIETY_GAIN * _e73) * ((2.0 * _e27.w) - 1.0));
                    }
                    float _e82 = gain;
                    float w = _e82 * (metal::exp(DOME_UNION * h) - 1.0);
                    float _e89 = weight;
                    weight = _e89 + w;
                    metal::float2 _e91 = face;
                    face = _e91 + ((w * -(((DOME_FACE * root) * radius))) * d);
                    metal::float2 _e99 = to_centre;
                    to_centre = _e99 + (w * (centre - r));
                }
            }
        }
    }
    float _e110 = weight;
    if (_e110 <= 0.0) {
        out.face = metal::float2(0.0);
        out.to_centre = metal::float2(0.0);
        Pile _e119 = out;
        return _e119;
    }
    metal::float2 _e121 = face;
    float _e122 = weight;
    out.face = _e121 / metal::float2(_e122);
    metal::float2 _e126 = to_centre;
    float _e127 = weight;
    out.to_centre = _e126 / metal::float2(_e127);
    Pile _e130 = out;
    return _e130;
}

int naga_f2i32(float value) {
    return static_cast<int>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

Pile cloud_domes(
    metal::float2 r_1,
    int period_2,
    constant Cloud& cloud
) {
    Pile out_1 = {};
    Pile _e2 = dome_octave(r_1, period_2, cloud);
    Pile _e14 = dome_octave((r_1 * DOME_LACUNARITY) + metal::float2(17.3, 5.9), naga_f2i32(metal::rint(DOME_LACUNARITY * static_cast<float>(period_2))), cloud);
    out_1.face = (_e2.face + (0.46199998 * _e14.face)) / metal::float2(1.22);
    out_1.to_centre = _e2.to_centre;
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
    metal::float2 uv = pt / _e3;
    metal::float4 _e8 = wide_light.sample(cloud_sampler, uv, metal::level(0.0));
    float _e10 = density_decode(_e8.x);
    metal::float4 _e14 = close_light.sample(cloud_sampler, uv, metal::level(0.0));
    float material = _e14.x;
    return 1.4 * metal::max(_e10, 0.85 * material);
}

float scale_tone(
    metal::float2 pt_1,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::sampler tile_sampler
) {
    Pile pile = {};
    metal::float2 _e3 = cloud.size;
    float _e10 = cloud.size.y;
    metal::float2 _e17 = cloud.drift;
    metal::float2 q_1 = (((pt_1 - (_e3 * 0.5)) / metal::float2(_e10)) * CLOUD_UNITS) + _e17;
    float _e22 = cloud.scale_size;
    float scale_units = SCALE_CELLS / _e22;
    float _e27 = cloud.size.y;
    float scale_points = (_e27 / CLOUD_UNITS) / scale_units;
    metal::float2 r_7 = q_1 * scale_units;
    uint _e35 = cloud.tile_cells;
    if (_e35 > 0u) {
        uint _e42 = cloud.tile_cells;
        metal::float4 tile = cloud_tile_a.sample(tile_sampler, r_7 / metal::float2(static_cast<float>(_e42)), metal::level(0.0));
        pile.face = tile.xy;
        pile.to_centre = tile.zw;
    } else {
        Pile _e53 = cloud_domes(r_7, 0, cloud);
        pile = _e53;
    }
    metal::float2 face_1 = pile.face;
    float _e58 = cloud.scale_refract;
    float bend = metal::max(_e58, 0.0) * scale_points;
    float _e64 = cloud.scale_refract;
    float gather = metal::max(-(_e64), 0.0) * scale_points;
    metal::float2 _e72 = pile.to_centre;
    metal::float2 lookup = (-(face_1) * bend) + (_e72 * gather);
    float _e76 = cloud_light(pt_1 + lookup, close_light, wide_light, cloud_sampler, cloud);
    metal::float2 reach = metal::float2(scale_points * 0.75, 0.0);
    float _e83 = cloud_light(pt_1 + reach.xy, close_light, wide_light, cloud_sampler, cloud);
    float _e86 = cloud_light(pt_1 - reach.xy, close_light, wide_light, cloud_sampler, cloud);
    float _e90 = cloud_light(pt_1 + reach.yx, close_light, wide_light, cloud_sampler, cloud);
    float _e93 = cloud_light(pt_1 - reach.yx, close_light, wide_light, cloud_sampler, cloud);
    metal::float2 grad = metal::float2(_e83 - _e86, _e90 - _e93);
    metal::float2 lean = grad * (SUN_LEAN / (SUN_KNEE + metal::length(grad)));
    float relief = cloud.scale_relief;
    metal::float3 normal = metal::normalize(metal::float3(-(face_1) * relief, 1.0));
    metal::float3 sun = metal::normalize(metal::float3(lean, 1.0));
    float lambert = metal::max(metal::dot(normal, sun), 0.0) / sun.z;
    float diffuse = metal::mix(metal::pow(1.0 - relief, RELIEF_FLOOR_FALL), 1.0, lambert);
    float lit = (_e76 * diffuse) * CLOUD_SHADE;
    return metal::clamp(lit, 0.0, 1.0);
}

metal::float3 wash_hash(
    metal::int2 cell_2,
    uint salt
) {
    uint n_1 = {};
    n_1 = (as_type<uint>(cell_2.x) * 2654435769u) ^ (as_type<uint>(cell_2.y) * 2246822507u);
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
    uint salt_1,
    int period_3
) {
    metal::float2 b = metal::floor(p);
    metal::float2 f_1 = p - b;
    metal::float2 t = (f_1 * f_1) * (metal::float2(3.0) - (2.0 * f_1));
    metal::int2 i_2 = naga_f2i32(b);
    metal::int2 _e13 = wrap_cell(i_2, period_3);
    metal::float3 _e14 = wash_hash(_e13, salt_1);
    float n00_ = _e14.x;
    metal::int2 _e20 = wrap_cell(as_type<metal::int2>(as_type<metal::uint2>(i_2) + as_type<metal::uint2>(metal::int2(1, 0))), period_3);
    metal::float3 _e21 = wash_hash(_e20, salt_1);
    float n10_ = _e21.x;
    metal::int2 _e27 = wrap_cell(as_type<metal::int2>(as_type<metal::uint2>(i_2) + as_type<metal::uint2>(metal::int2(0, 1))), period_3);
    metal::float3 _e28 = wash_hash(_e27, salt_1);
    float n01_ = _e28.x;
    metal::int2 _e34 = wrap_cell(as_type<metal::int2>(as_type<metal::uint2>(i_2) + as_type<metal::uint2>(metal::int2(1, 1))), period_3);
    metal::float3 _e35 = wash_hash(_e34, salt_1);
    float n11_ = _e35.x;
    return metal::mix(metal::mix(n00_, n10_, t.x), metal::mix(n01_, n11_, t.x), t.y);
}

float wash_fbm(
    metal::float2 p_1,
    uint salt_2,
    int period_4
) {
    float lacunarity = (period_4 > 0) ? WASH_FBM_FINE_TILED : WASH_FBM_FINE;
    float _e8 = wash_noise(p_1, salt_2, period_4);
    float _e18 = wash_noise((p_1 * lacunarity) + metal::float2(13.1, -7.3), salt_2 + 31u, as_type<int>(as_type<uint>(period_4) * as_type<uint>(2)));
    return (_e8 + (0.5 * _e18)) / 1.5;
}

Glob wash_glob(
    metal::int2 cell_3,
    uint salt_3,
    metal::float2 r_2,
    float wob,
    float occupancy,
    int period_5
) {
    Glob out_2 = {};
    metal::int2 _e6 = wrap_cell(cell_3, period_5);
    metal::float3 _e9 = wash_hash(_e6, salt_3 + 77u);
    out_2.order = _e9.x;
    if (_e9.y > occupancy) {
        out_2.centre = r_2;
        out_2.edge = 1000000000.0;
        Glob _e18 = out_2;
        return _e18;
    }
    metal::float3 _e19 = wash_hash(_e6, salt_3);
    metal::float2 centre_1 = (static_cast<metal::float2>(cell_3) + metal::float2(0.5)) + ((_e19.xy - metal::float2(0.5)) * WASH_JITTER);
    float radius_1 = metal::mix(WASH_RADIUS_MIN, WASH_RADIUS_MAX, _e19.z);
    out_2.centre = centre_1;
    out_2.edge = (metal::length(r_2 - centre_1) / radius_1) + wob;
    Glob _e41 = out_2;
    return _e41;
}

Wash wash_scan(
    metal::float2 r_3,
    uint salt_4,
    float occupancy_1,
    float wob_1,
    int period_6
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
    front_a = r_3;
    front_b = r_3;
    metal::int2 base_1 = naga_f2i32(metal::floor(r_3));
    uint2 loop_bound_2 = uint2(4294967295u);
    bool loop_init_2 = true;
    while(true) {
        if (metal::all(loop_bound_2 == uint2(0u))) { break; }
        loop_bound_2 -= uint2(loop_bound_2.y == 0u, 1u);
        if (!loop_init_2) {
            int _e93 = j_1;
            j_1 = as_type<int>(as_type<uint>(_e93) + as_type<uint>(1));
        }
        loop_init_2 = false;
        int _e33 = j_1;
        if (_e33 <= WASH_RING) {
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
                    int _e90 = i_1;
                    i_1 = as_type<int>(as_type<uint>(_e90) + as_type<uint>(1));
                }
                loop_init_3 = false;
                int _e38 = i_1;
                if (_e38 <= WASH_RING) {
                } else {
                    break;
                }
                {
                    int _e41 = i_1;
                    int _e42 = j_1;
                    Glob _e45 = wash_glob(as_type<metal::int2>(as_type<metal::uint2>(base_1) + as_type<metal::uint2>(metal::int2(_e41, _e42))), salt_4, r_3, wob_1, occupancy_1, period_6);
                    float prox = 1.0 - _e45.edge;
                    float _e51 = out_3.cover;
                    out_3.cover = metal::max(_e51, metal::clamp(prox / 0.05, 0.0, 1.0));
                    if (_e45.edge < 1.0) {
                        float _e62 = best;
                        if (_e45.order > _e62) {
                            float _e64 = best;
                            second = _e64;
                            metal::float2 _e67 = out_3.centre;
                            out_3.under = _e67;
                            best = _e45.order;
                            out_3.centre = _e45.centre;
                            out_3.edge = _e45.edge;
                        } else {
                            float _e74 = second;
                            if (_e45.order > _e74) {
                                second = _e45.order;
                                out_3.under = _e45.centre;
                            }
                        }
                    } else {
                        float _e79 = near_a;
                        if (prox > _e79) {
                            float _e81 = near_a;
                            near_b = _e81;
                            float _e82 = order_a;
                            order_b = _e82;
                            metal::float2 _e83 = front_a;
                            front_b = _e83;
                            near_a = prox;
                            order_a = _e45.order;
                            front_a = _e45.centre;
                        } else {
                            float _e86 = near_b;
                            if (prox > _e86) {
                                near_b = prox;
                                order_b = _e45.order;
                                front_b = _e45.centre;
                            }
                        }
                    }
                }
            }
        }
    }
    float _e96 = order_b;
    float _e97 = best;
    if (_e96 > _e97) {
        float _e100 = near_b;
        out_3.near = _e100;
        metal::float2 _e102 = front_b;
        out_3.front = _e102;
    }
    float _e103 = order_a;
    float _e104 = best;
    if (_e103 > _e104) {
        float _e107 = near_a;
        out_3.near = _e107;
        metal::float2 _e109 = front_a;
        out_3.front = _e109;
    }
    Wash _e110 = out_3;
    return _e110;
}

float wash_light(
    metal::float2 pt_2,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    metal::float2 _e5 = cloud.size;
    metal::float4 _e8 = close_light.sample(cloud_sampler, pt_2 / _e5, metal::level(0.0));
    return _e8.x;
}

Wet wash_wet(
    Wash f,
    metal::float2 r_4,
    constant Cloud& cloud
) {
    metal::float2 look = {};
    float fa = {};
    float bl = {};
    float pigment = {};
    float fuzz = cloud.wash_fuzz;
    float feather = 0.1 + (0.8 * fuzz);
    float bleed = 0.12 + (0.78 * fuzz);
    float _e16 = cloud.wash_pool;
    float tide = (WASH_POOL * _e16) * (1.0 - (0.75 * fuzz));
    float surf = WASH_SURF * (1.0 - (0.45 * fuzz));
    look = f.centre;
    fa = metal::clamp((f.edge - (1.0 - feather)) / feather, 0.0, 1.0);
    float _e40 = fa;
    float _e41 = fa;
    float _e43 = fa;
    fa = ((_e40 * _e41) * (3.0 - (2.0 * _e43))) * 0.5;
    metal::float2 _e51 = look;
    float _e53 = fa;
    look = metal::mix(_e51, f.under, _e53);
    bl = metal::clamp((f.near + bleed) / bleed, 0.0, 1.0);
    float _e62 = bl;
    float _e63 = bl;
    float _e65 = bl;
    bl = ((_e62 * _e63) * (3.0 - (2.0 * _e65))) * 0.5;
    metal::float2 _e73 = look;
    float _e75 = bl;
    look = metal::mix(_e73, f.front, _e75);
    float rim = metal::clamp(f.edge, 0.0, 1.0);
    pigment = (surf * rim) * rim;
    float crescent = metal::clamp((f.near + WASH_POOL_WIDTH) / WASH_POOL_WIDTH, 0.0, 1.0);
    float _e92 = pigment;
    pigment = _e92 + ((tide * crescent) * crescent);
    metal::float2 _e96 = look;
    float _e98 = pigment;
    return Wet {_e96 - r_4, metal::max(_e98, 0.0)};
}

Painted wash_paint(
    Wet wet,
    float pane_per_cell,
    metal::float2 pt_3,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    float _e7 = cloud.wash_refract;
    float _e10 = wash_light(pt_3 + ((wet.offset * pane_per_cell) * _e7), close_light, cloud_sampler, cloud);
    float paper = metal::clamp((WASH_PIVOT + (WASH_LIFT_A * (_e10 - WASH_PIVOT))) + WASH_LIFT_B, 0.0, 1.0);
    float tone = paper - (wet.pigment * (WASH_PIG_DEPTH + (0.65 * paper)));
    return Painted {tone, metal::smoothstep(0.0, WASH_BLACK_KNEE, _e10)};
}

WashField wash_field(
    metal::float2 r_5,
    int period_7,
    bool want_fine,
    constant Cloud& cloud
) {
    metal::float2 warped = {};
    float wob_2 = 0.0;
    WashField out_4 = {};
    warped = r_5;
    float _e6 = cloud.wash_lobe;
    if (_e6 > 0.0) {
        float _e12 = cloud.wash_lobe;
        float amp = WASH_WARP * _e12;
        int warp_period = naga_f2i32(metal::rint(WASH_WARP_SCALE * static_cast<float>(period_7)));
        metal::float2 _e19 = warped;
        float _e25 = wash_fbm(r_5 * WASH_WARP_SCALE, 71u, warp_period);
        float _e35 = wash_fbm((r_5 * WASH_WARP_SCALE) + metal::float2(37.0, -19.0), 73u, warp_period);
        warped = _e19 + ((amp * 2.0) * metal::float2(_e25 - 0.5, _e35 - 0.5));
    }
    float _e45 = cloud.wash_ragged;
    if (_e45 > 0.0) {
        int ragged_period = naga_f2i32(metal::rint(WASH_RAGGED_SCALE * static_cast<float>(period_7)));
        float _e56 = cloud.wash_ragged;
        float _e61 = wash_fbm(r_5 * WASH_RAGGED_SCALE, 41u, ragged_period);
        wob_2 = (WASH_RAGGED * _e56) * (_e61 - 1.0);
    }
    metal::float2 _e67 = warped;
    float _e70 = wob_2;
    Wash _e71 = wash_scan(_e67, 1u, 1.0, _e70, period_7);
    metal::float2 _e72 = warped;
    Wet _e73 = wash_wet(_e71, _e72, cloud);
    out_4.coarse = _e73;
    out_4.fine = Wet {metal::float2(0.0), 0.0};
    out_4.cover = 0.0;
    if (want_fine) {
        metal::float2 _e81 = warped;
        metal::float2 fine_r = (_e81 * WASH_LACUNARITY) + metal::float2(17.3, 5.9);
        float _e90 = wob_2;
        Wash _e96 = wash_scan(fine_r, 2u, WASH_FINE_OCCUPANCY, _e90, naga_f2i32(metal::rint(WASH_LACUNARITY * static_cast<float>(period_7))));
        Wet _e98 = wash_wet(_e96, fine_r, cloud);
        out_4.fine = _e98;
        out_4.cover = _e96.cover;
    }
    WashField _e101 = out_4;
    return _e101;
}

WashField wash_tile_field(
    metal::float2 r_6,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    WashField out_5 = {};
    uint _e3 = cloud.tile_cells;
    metal::float2 uv_1 = r_6 / metal::float2(static_cast<float>(_e3));
    metal::float4 a = cloud_tile_a.sample(tile_sampler, uv_1, metal::level(0.0));
    out_5.coarse = Wet {a.xy, a.z};
    out_5.fine = Wet {metal::float2(0.0), 0.0};
    out_5.cover = 0.0;
    float _e25 = cloud.wash_layers;
    if (_e25 > 0.0) {
        metal::float4 b_1 = cloud_tile_b.sample(tile_sampler, uv_1, metal::level(0.0));
        out_5.fine = Wet {b_1.xy, b_1.z};
        out_5.cover = b_1.w;
    }
    WashField _e38 = out_5;
    return _e38;
}

float wash_cloud_tone(
    metal::float2 pt_4,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    WashField field = {};
    Painted paint = {};
    metal::float2 _e3 = cloud.size;
    float _e10 = cloud.size.y;
    metal::float2 _e17 = cloud.drift;
    metal::float2 q_2 = (((pt_4 - (_e3 * 0.5)) / metal::float2(_e10)) * CLOUD_UNITS) + _e17;
    float _e22 = cloud.wash_size;
    float cells = WASH_CELLS / _e22;
    float _e27 = cloud.size.y;
    float pane_per_cell_1 = (_e27 / CLOUD_UNITS) / cells;
    metal::float2 r_8 = q_2 * cells;
    uint _e35 = cloud.tile_cells;
    if (_e35 > 0u) {
        WashField _e38 = wash_tile_field(r_8, cloud, cloud_tile_a, cloud_tile_b, tile_sampler);
        field = _e38;
    } else {
        float _e42 = cloud.wash_layers;
        WashField _e45 = wash_field(r_8, 0, _e42 > 0.0, cloud);
        field = _e45;
    }
    Wet _e47 = field.coarse;
    Painted _e48 = wash_paint(_e47, pane_per_cell_1, pt_4, close_light, cloud_sampler, cloud);
    paint = _e48;
    float _e52 = cloud.wash_layers;
    if (_e52 > 0.0) {
        Wet _e56 = field.fine;
        Painted _e59 = wash_paint(_e56, pane_per_cell_1 / WASH_LACUNARITY, pt_4, close_light, cloud_sampler, cloud);
        float _e62 = cloud.wash_layers;
        float _e64 = field.cover;
        float over = _e62 * _e64;
        float _e68 = paint.tone;
        paint.tone = metal::mix(_e68, _e59.tone, over);
        float _e73 = paint.hold;
        paint.hold = metal::mix(_e73, _e59.hold, over);
    }
    float _e77 = paint.tone;
    float _e82 = paint.hold;
    return metal::clamp(_e77, WASH_TONE_FLOOR, 1.0) * _e82;
}

float cloud_tone_at(
    metal::float2 pt_5,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    uint _e3 = cloud.cloud_style;
    if (_e3 == 1u) {
        float _e6 = wash_cloud_tone(pt_5, close_light, cloud_sampler, cloud, cloud_tile_a, cloud_tile_b, tile_sampler);
        return _e6;
    }
    float _e7 = scale_tone(pt_5, close_light, wide_light, cloud_sampler, cloud, cloud_tile_a, tile_sampler);
    return _e7;
}

struct fs_cloud_toneInput {
    float slab [[user(loc0), center_perspective]];
    float t [[user(loc1), center_perspective]];
};
struct fs_cloud_toneOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_cloud_toneOutput fs_cloud_tone(
  fs_cloud_toneInput varyings [[stage_in]]
, metal::float4 position [[position]]
, metal::texture2d<float, metal::access::sample> close_light [[texture(1)]]
, metal::texture2d<float, metal::access::sample> wide_light [[texture(2)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(2)]]
, metal::texture2d<float, metal::access::sample> cloud_tile_a [[texture(4)]]
, metal::texture2d<float, metal::access::sample> cloud_tile_b [[texture(5)]]
, metal::sampler tile_sampler [[sampler(1)]]
) {
    const VertexOut in = { position, varyings.slab, varyings.t };
    metal::float2 _e6 = cloud.size;
    float _e8 = cloud_tone_at(metal::float2(in.slab, in.t) * _e6, close_light, wide_light, cloud_sampler, cloud, cloud_tile_a, cloud_tile_b, tile_sampler);
    return fs_cloud_toneOutput { metal::float4(_e8, 0.0, 0.0, 1.0) };
}
