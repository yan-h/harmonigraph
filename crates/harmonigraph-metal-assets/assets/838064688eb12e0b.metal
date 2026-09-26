// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct StarSlice {
    metal::float2 offset;
    float cell;
    float sigma;
    float cap;
    float defocus;
    float occupancy;
    float blur;
    float fringe;
    float fringe_reach;
    float reach;
    float unseen;
    metal::float2 spread;
    float life;
    float _pad;
};
struct type_5 {
    StarSlice inner[5];
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
    uint cloud_style;
    float wash_size;
    float wash_fuzz;
    float wash_lobe;
    float wash_refract;
    float wash_layers;
    uint tile_cells;
    uint pitch_vertical;
    float star_randomness;
    float star_glow;
    float star_wander;
    float star_time;
    metal::float2 _star_pad;
    type_5 star_slices;
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
struct Wet {
    metal::float2 offset;
};
struct WashField {
    Wet coarse;
    Wet fine;
    float cover;
    char _pad3[4];
};
struct TileVertex {
    metal::float4 position;
    metal::float2 fraction;
    char _pad2[8];
};
struct TileBake {
    metal::float4 a;
    metal::float4 b;
};
constant float CLOUD_UNITS = 10.0;
constant float CLOUD_TILE_ROT_COS = 0.8;
constant float CLOUD_TILE_ROT_SIN = 0.6;
constant float SCALE_CELLS = 2.7272727;
constant float DOME_RADIUS = 1.15;
constant float DOME_JITTER = 0.3;
constant float DOME_RADIUS_MIN = 0.95;
constant float DOME_RADIUS_MAX = 1.32;
constant float DOME_UNION = 9.0;
constant float DOME_VARIETY_GAIN = 5.0;
constant float DOME_FACE = 1.5122874;
constant float DOME_LACUNARITY = 2.1;
constant float DOME_FINE_GAIN = 0.22;
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
constant uint STAR_SLICES = 5u;
constant float STAR_PANE = 540.0;
constant float STAR_JITTER = 0.6;
constant int STAR_HASH_PERIOD = 4096;
constant float STAR_WANDER_PERIOD = 400.0;
constant uint STAR_LIFE_PERIOD = 4096u;
constant float STAR_FADE = 0.2;
constant float STAR_EXPOSURE = 1.5;
constant float STAR_LIFT = 0.18;
constant float STAR_OVER_GROUND = 6.0;
constant float STAR_RING_FADE = 0.7;
constant float STAR_TAU = 6.2831855;

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
    constant Cloud& cloud
) {
    float weight = 0.0;
    metal::float2 face = metal::float2(0.0);
    metal::float2 to_centre = metal::float2(0.0);
    int j = -1;
    int i = {};
    float gain = {};
    Pile out_1 = {};
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
                    metal::int2 cell_5 = as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(base)) + as_type<metal::uint2>(metal::int2(_e22, _e23)));
                    metal::int2 _e26 = wrap_cell(cell_5, period_2);
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
        out_1.face = metal::float2(0.0);
        out_1.to_centre = metal::float2(0.0);
        Pile _e119 = out_1;
        return _e119;
    }
    metal::float2 _e121 = face;
    float _e122 = weight;
    out_1.face = _e121 / metal::float2(_e122);
    metal::float2 _e126 = to_centre;
    float _e127 = weight;
    out_1.to_centre = _e126 / metal::float2(_e127);
    Pile _e130 = out_1;
    return _e130;
}

int naga_f2i32(float value) {
    return static_cast<int>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

Pile cloud_domes(
    metal::float2 r_1,
    int period_3,
    constant Cloud& cloud
) {
    Pile out_2 = {};
    Pile _e2 = dome_octave(r_1, period_3, cloud);
    Pile _e14 = dome_octave((r_1 * DOME_LACUNARITY) + metal::float2(17.3, 5.9), naga_f2i32(metal::rint(DOME_LACUNARITY * static_cast<float>(period_3))), cloud);
    out_2.face = (_e2.face + (0.46199998 * _e14.face)) / metal::float2(1.22);
    out_2.to_centre = _e2.to_centre;
    Pile _e27 = out_2;
    return _e27;
}

metal::float3 wash_hash(
    metal::int2 cell_3,
    uint salt
) {
    uint n_1 = {};
    n_1 = (as_type<uint>(cell_3.x) * 2654435769u) ^ (as_type<uint>(cell_3.y) * 2246822507u);
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
    int period_4
) {
    metal::float2 b = metal::floor(p);
    metal::float2 f_1 = p - b;
    metal::float2 t = (f_1 * f_1) * (metal::float2(3.0) - (2.0 * f_1));
    metal::int2 i_2 = naga_f2i32(b);
    metal::int2 _e13 = wrap_cell(i_2, period_4);
    metal::float3 _e14 = wash_hash(_e13, salt_1);
    float n00_ = _e14.x;
    metal::int2 _e20 = wrap_cell(as_type<metal::int2>(as_type<metal::uint2>(i_2) + as_type<metal::uint2>(metal::int2(1, 0))), period_4);
    metal::float3 _e21 = wash_hash(_e20, salt_1);
    float n10_ = _e21.x;
    metal::int2 _e27 = wrap_cell(as_type<metal::int2>(as_type<metal::uint2>(i_2) + as_type<metal::uint2>(metal::int2(0, 1))), period_4);
    metal::float3 _e28 = wash_hash(_e27, salt_1);
    float n01_ = _e28.x;
    metal::int2 _e34 = wrap_cell(as_type<metal::int2>(as_type<metal::uint2>(i_2) + as_type<metal::uint2>(metal::int2(1, 1))), period_4);
    metal::float3 _e35 = wash_hash(_e34, salt_1);
    float n11_ = _e35.x;
    return metal::mix(metal::mix(n00_, n10_, t.x), metal::mix(n01_, n11_, t.x), t.y);
}

float wash_fbm(
    metal::float2 p_1,
    uint salt_2,
    int period_5
) {
    float lacunarity = (period_5 > 0) ? WASH_FBM_FINE_TILED : WASH_FBM_FINE;
    float _e8 = wash_noise(p_1, salt_2, period_5);
    float _e18 = wash_noise((p_1 * lacunarity) + metal::float2(13.1, -7.3), salt_2 + 31u, as_type<int>(as_type<uint>(period_5) * as_type<uint>(2)));
    return (_e8 + (0.5 * _e18)) / 1.5;
}

Glob wash_glob(
    metal::int2 cell_4,
    uint salt_3,
    metal::float2 r_2,
    float occupancy,
    int period_6
) {
    Glob out_3 = {};
    metal::int2 _e5 = wrap_cell(cell_4, period_6);
    metal::float3 _e8 = wash_hash(_e5, salt_3 + 77u);
    out_3.order = _e8.x;
    if (_e8.y > occupancy) {
        out_3.centre = r_2;
        out_3.edge = 1000000000.0;
        Glob _e17 = out_3;
        return _e17;
    }
    metal::float3 _e18 = wash_hash(_e5, salt_3);
    metal::float2 centre_1 = (static_cast<metal::float2>(cell_4) + metal::float2(0.5)) + ((_e18.xy - metal::float2(0.5)) * WASH_JITTER);
    float radius_1 = metal::mix(WASH_RADIUS_MIN, WASH_RADIUS_MAX, _e18.z);
    out_3.centre = centre_1;
    out_3.edge = metal::length(r_2 - centre_1) / radius_1;
    Glob _e39 = out_3;
    return _e39;
}

Wash wash_scan(
    metal::float2 r_3,
    uint salt_4,
    float occupancy_1,
    int period_7
) {
    Wash out_4 = {};
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
    out_4.centre = r_3;
    out_4.under = r_3;
    out_4.front = r_3;
    out_4.near = -1000000000.0;
    out_4.edge = 1.0;
    out_4.cover = 0.0;
    front_a = r_3;
    front_b = r_3;
    metal::int2 base_1 = naga_f2i32(metal::floor(r_3));
    uint2 loop_bound_2 = uint2(4294967295u);
    bool loop_init_2 = true;
    while(true) {
        if (metal::all(loop_bound_2 == uint2(0u))) { break; }
        loop_bound_2 -= uint2(loop_bound_2.y == 0u, 1u);
        if (!loop_init_2) {
            int _e92 = j_1;
            j_1 = as_type<int>(as_type<uint>(_e92) + as_type<uint>(1));
        }
        loop_init_2 = false;
        int _e32 = j_1;
        if (_e32 <= WASH_RING) {
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
                    int _e89 = i_1;
                    i_1 = as_type<int>(as_type<uint>(_e89) + as_type<uint>(1));
                }
                loop_init_3 = false;
                int _e37 = i_1;
                if (_e37 <= WASH_RING) {
                } else {
                    break;
                }
                {
                    int _e40 = i_1;
                    int _e41 = j_1;
                    Glob _e44 = wash_glob(as_type<metal::int2>(as_type<metal::uint2>(base_1) + as_type<metal::uint2>(metal::int2(_e40, _e41))), salt_4, r_3, occupancy_1, period_7);
                    float prox = 1.0 - _e44.edge;
                    float _e50 = out_4.cover;
                    out_4.cover = metal::max(_e50, metal::clamp(prox / 0.05, 0.0, 1.0));
                    if (_e44.edge < 1.0) {
                        float _e61 = best;
                        if (_e44.order > _e61) {
                            float _e63 = best;
                            second = _e63;
                            metal::float2 _e66 = out_4.centre;
                            out_4.under = _e66;
                            best = _e44.order;
                            out_4.centre = _e44.centre;
                            out_4.edge = _e44.edge;
                        } else {
                            float _e73 = second;
                            if (_e44.order > _e73) {
                                second = _e44.order;
                                out_4.under = _e44.centre;
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
        out_4.near = _e99;
        metal::float2 _e101 = front_b;
        out_4.front = _e101;
    }
    float _e102 = order_a;
    float _e103 = best;
    if (_e102 > _e103) {
        float _e106 = near_a;
        out_4.near = _e106;
        metal::float2 _e108 = front_a;
        out_4.front = _e108;
    }
    Wash _e109 = out_4;
    return _e109;
}

Wet wash_wet(
    Wash f,
    metal::float2 r_4,
    constant Cloud& cloud
) {
    metal::float2 look = {};
    float fa = {};
    float bl = {};
    float _e4 = cloud.wash_fuzz;
    float feather = 0.1 + (0.8 * _e4);
    float _e11 = cloud.wash_fuzz;
    float bleed = 0.12 + (0.78 * _e11);
    look = f.centre;
    fa = metal::clamp((f.edge - (1.0 - feather)) / feather, 0.0, 1.0);
    float _e27 = fa;
    float _e28 = fa;
    float _e30 = fa;
    fa = ((_e27 * _e28) * (3.0 - (2.0 * _e30))) * 0.5;
    metal::float2 _e38 = look;
    float _e40 = fa;
    look = metal::mix(_e38, f.under, _e40);
    bl = metal::clamp((f.near + bleed) / bleed, 0.0, 1.0);
    float _e49 = bl;
    float _e50 = bl;
    float _e52 = bl;
    bl = ((_e49 * _e50) * (3.0 - (2.0 * _e52))) * 0.5;
    metal::float2 _e60 = look;
    float _e62 = bl;
    look = metal::mix(_e60, f.front, _e62);
    metal::float2 _e64 = look;
    return Wet {_e64 - r_4};
}

WashField wash_field(
    metal::float2 r_5,
    int period_8,
    constant Cloud& cloud
) {
    metal::float2 warped = {};
    WashField out_5 = {};
    warped = r_5;
    float _e5 = cloud.wash_lobe;
    if (_e5 > 0.0) {
        float _e11 = cloud.wash_lobe;
        float amp = WASH_WARP * _e11;
        int warp_period = naga_f2i32(metal::rint(WASH_WARP_SCALE * static_cast<float>(period_8)));
        metal::float2 _e18 = warped;
        float _e24 = wash_fbm(r_5 * WASH_WARP_SCALE, 71u, warp_period);
        float _e34 = wash_fbm((r_5 * WASH_WARP_SCALE) + metal::float2(37.0, -19.0), 73u, warp_period);
        warped = _e18 + ((amp * 2.0) * metal::float2(_e24 - 0.5, _e34 - 0.5));
    }
    metal::float2 _e42 = warped;
    Wash _e45 = wash_scan(_e42, 1u, 1.0, period_8);
    metal::float2 _e46 = warped;
    Wet _e47 = wash_wet(_e45, _e46, cloud);
    out_5.coarse = _e47;
    metal::float2 _e48 = warped;
    metal::float2 fine_r = (_e48 * WASH_LACUNARITY) + metal::float2(17.3, 5.9);
    Wash _e62 = wash_scan(fine_r, 2u, WASH_FINE_OCCUPANCY, naga_f2i32(metal::rint(WASH_LACUNARITY * static_cast<float>(period_8))));
    Wet _e64 = wash_wet(_e62, fine_r, cloud);
    out_5.fine = _e64;
    out_5.cover = _e62.cover;
    WashField _e67 = out_5;
    return _e67;
}

struct fs_cloud_tileInput {
    metal::float2 fraction [[user(loc0), center_perspective]];
};
struct fs_cloud_tileOutput {
    metal::float4 a [[color(0)]];
    metal::float4 b [[color(1)]];
};
fragment fs_cloud_tileOutput fs_cloud_tile(
  fs_cloud_tileInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Cloud& cloud [[buffer(2)]]
) {
    const TileVertex in = { position, varyings.fraction };
    TileBake out = {};
    uint _e3 = cloud.tile_cells;
    int period_9 = static_cast<int>(_e3);
    uint _e7 = cloud.tile_cells;
    float period_f = static_cast<float>(_e7);
    float time = in.fraction.x * period_f;
    float pitch = in.fraction.y * period_f;
    uint _e19 = cloud.pitch_vertical;
    metal::float2 wash_cell = (_e19 == 1u) ? metal::float2(time, pitch) : metal::float2(pitch, time);
    metal::float2 mosaic_cell = in.fraction * period_f;
    out.a = metal::float4(0.0);
    out.b = metal::float4(0.0);
    uint _e34 = cloud.cloud_style;
    if (_e34 == 1u) {
        WashField _e37 = wash_field(wash_cell, period_9, cloud);
        out.a = metal::float4(_e37.coarse.offset, 0.0, 0.0);
        out.b = metal::float4(_e37.fine.offset, 0.0, _e37.cover);
    } else {
        Pile _e50 = cloud_domes(mosaic_cell, period_9, cloud);
        out.a = metal::float4(_e50.face, _e50.to_centre);
    }
    TileBake _e55 = out;
    const auto _tmp = _e55;
    return fs_cloud_tileOutput { _tmp.a, _tmp.b };
}
