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
struct StarSlice {
    metal::float2 offset;
    float cell;
    float sigma;
    float cap;
    float defocus;
    float fringe;
    int base;
    metal::int2 origin;
    metal::int2 grid;
};
struct type_7 {
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
    float star_life;
    type_7 star_slices;
    uint memory_enabled;
    uint memory_valid;
    float pickup_alpha;
    float release_alpha;
    metal::int2 memory_shift;
    metal::float2 memory_fraction;
    float previous_life;
    float memory_pad_a;
    metal::float2 memory_extent;
    type_7 previous_slices;
};
struct Pile {
    metal::float2 face;
    metal::float2 to_centre;
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
constant float CLOUD_UNITS = 10.0;
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
constant uint STAR_SLICES = 5u;
constant float STAR_PANE = 540.0;
constant float STAR_JITTER = 0.6;
constant float STAR_REACH = 1.2;
constant int STAR_ATLAS_WIDTH = 2048;
constant uint STAR_ATLAS_SHIFT = 11u;
constant int STAR_HASH_PERIOD = 65536;
constant uint STAR_LIFE_PERIOD = 4096u;
constant float STAR_FADE = 0.2;
constant float STAR_LIFT = 0.18;
constant float STAR_RING_FADE = 0.7;

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
    metal::int2 i_2 = naga_f2i32(b);
    metal::int2 _e13 = wrap_cell(i_2, period_2);
    metal::float3 _e14 = wash_hash(_e13, salt_1);
    float n00_ = _e14.x;
    metal::int2 _e20 = wrap_cell(as_type<metal::int2>(as_type<metal::uint2>(i_2) + as_type<metal::uint2>(metal::int2(1, 0))), period_2);
    metal::float3 _e21 = wash_hash(_e20, salt_1);
    float n10_ = _e21.x;
    metal::int2 _e27 = wrap_cell(as_type<metal::int2>(as_type<metal::uint2>(i_2) + as_type<metal::uint2>(metal::int2(0, 1))), period_2);
    metal::float3 _e28 = wash_hash(_e27, salt_1);
    float n01_ = _e28.x;
    metal::int2 _e34 = wrap_cell(as_type<metal::int2>(as_type<metal::uint2>(i_2) + as_type<metal::uint2>(metal::int2(1, 1))), period_2);
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
    Glob out_1 = {};
    metal::int2 _e5 = wrap_cell(cell_3, period_4);
    metal::float3 _e8 = wash_hash(_e5, salt_3 + 77u);
    out_1.order = _e8.x;
    if (_e8.y > occupancy) {
        out_1.centre = r;
        out_1.edge = 1000000000.0;
        Glob _e17 = out_1;
        return _e17;
    }
    metal::float3 _e18 = wash_hash(_e5, salt_3);
    metal::float2 centre = (static_cast<metal::float2>(cell_3) + metal::float2(0.5)) + ((_e18.xy - metal::float2(0.5)) * WASH_JITTER);
    float radius = metal::mix(WASH_RADIUS_MIN, WASH_RADIUS_MAX, _e18.z);
    out_1.centre = centre;
    out_1.edge = metal::length(r - centre) / radius;
    Glob _e39 = out_1;
    return _e39;
}

Wash wash_scan(
    metal::float2 r_1,
    uint salt_4,
    float occupancy_1,
    int period_5
) {
    Wash out_2 = {};
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
    out_2.centre = r_1;
    out_2.under = r_1;
    out_2.front = r_1;
    out_2.near = -1000000000.0;
    out_2.edge = 1.0;
    out_2.cover = 0.0;
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
                    float _e50 = out_2.cover;
                    out_2.cover = metal::max(_e50, metal::clamp(prox / 0.05, 0.0, 1.0));
                    if (_e44.edge < 1.0) {
                        float _e61 = best;
                        if (_e44.order > _e61) {
                            float _e63 = best;
                            second = _e63;
                            metal::float2 _e66 = out_2.centre;
                            out_2.under = _e66;
                            best = _e44.order;
                            out_2.centre = _e44.centre;
                            out_2.edge = _e44.edge;
                        } else {
                            float _e73 = second;
                            if (_e44.order > _e73) {
                                second = _e44.order;
                                out_2.under = _e44.centre;
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
        out_2.near = _e99;
        metal::float2 _e101 = front_b;
        out_2.front = _e101;
    }
    float _e102 = order_a;
    float _e103 = best;
    if (_e102 > _e103) {
        float _e106 = near_a;
        out_2.near = _e106;
        metal::float2 _e108 = front_a;
        out_2.front = _e108;
    }
    Wash _e109 = out_2;
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
    WashField out_3 = {};
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
    out_3.coarse = _e43;
    metal::float2 _e44 = warped;
    metal::float2 fine_r = (_e44 * WASH_LACUNARITY) + metal::float2(17.3, 5.9);
    Wash _e58 = wash_scan(fine_r, 2u, WASH_FINE_OCCUPANCY, naga_f2i32(metal::rint(WASH_LACUNARITY * static_cast<float>(period_6))));
    Wet _e60 = wash_wet(_e58, fine_r, fuzz_1);
    out_3.fine = _e60;
    out_3.cover = _e58.cover;
    WashField _e63 = out_3;
    return _e63;
}

metal::float4 cloud_hash4_(
    metal::int2 cell_4
) {
    uint n_1 = {};
    uint m = {};
    n_1 = (as_type<uint>(cell_4.x) * 2654435769u) ^ (as_type<uint>(cell_4.y) * 2246822507u);
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
    m = (_e30 ^ 3039394381u) * 1759714724u;
    uint _e36 = m;
    uint _e37 = m;
    m = _e36 ^ (_e37 >> 15u);
    uint _e41 = n_1;
    uint _e47 = n_1;
    uint _e55 = n_1;
    uint _e63 = m;
    return metal::float4(static_cast<float>(_e41 & 1023u) / 1023.0, static_cast<float>((_e47 >> 10u) & 1023u) / 1023.0, static_cast<float>((_e55 >> 20u) & 1023u) / 1023.0, static_cast<float>(_e63 & 1023u) / 1023.0);
}

Pile dome_octave(
    metal::float2 r_4,
    int period_7,
    constant Cloud& cloud
) {
    float weight = 0.0;
    metal::float2 face = metal::float2(0.0);
    metal::float2 to_centre = metal::float2(0.0);
    int j_1 = -1;
    int i_1 = {};
    float gain = {};
    Pile out_4 = {};
    metal::float2 base_1 = metal::floor(r_4);
    uint2 loop_bound_2 = uint2(4294967295u);
    bool loop_init_2 = true;
    while(true) {
        if (metal::all(loop_bound_2 == uint2(0u))) { break; }
        loop_bound_2 -= uint2(loop_bound_2.y == 0u, 1u);
        if (!loop_init_2) {
            int _e106 = j_1;
            j_1 = as_type<int>(as_type<uint>(_e106) + as_type<uint>(1));
        }
        loop_init_2 = false;
        int _e13 = j_1;
        if (_e13 <= 1) {
        } else {
            break;
        }
        {
            i_1 = -1;
            uint2 loop_bound_3 = uint2(4294967295u);
            bool loop_init_3 = true;
            while(true) {
                if (metal::all(loop_bound_3 == uint2(0u))) { break; }
                loop_bound_3 -= uint2(loop_bound_3.y == 0u, 1u);
                if (!loop_init_3) {
                    int _e103 = i_1;
                    i_1 = as_type<int>(as_type<uint>(_e103) + as_type<uint>(1));
                }
                loop_init_3 = false;
                int _e18 = i_1;
                if (_e18 <= 1) {
                } else {
                    break;
                }
                {
                    int _e22 = i_1;
                    int _e23 = j_1;
                    metal::int2 cell_5 = as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(base_1)) + as_type<metal::uint2>(metal::int2(_e22, _e23)));
                    metal::int2 _e26 = wrap_cell(cell_5, period_7);
                    metal::float4 _e27 = cloud_hash4_(_e26);
                    int _e28 = i_1;
                    int _e30 = j_1;
                    metal::float2 centre_1 = ((base_1 + metal::float2(static_cast<float>(_e28), static_cast<float>(_e30))) + metal::float2(0.5)) + ((_e27.xy - metal::float2(0.5)) * DOME_JITTER);
                    float _e51 = cloud.scale_variety;
                    float radius_1 = metal::mix(DOME_RADIUS, metal::mix(DOME_RADIUS_MIN, DOME_RADIUS_MAX, _e27.z), _e51);
                    metal::float2 d = (r_4 - centre_1) / metal::float2(radius_1);
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
                    face = _e91 + ((w * -(((DOME_FACE * root) * radius_1))) * d);
                    metal::float2 _e99 = to_centre;
                    to_centre = _e99 + (w * (centre_1 - r_4));
                }
            }
        }
    }
    float _e110 = weight;
    if (_e110 <= 0.0) {
        out_4.face = metal::float2(0.0);
        out_4.to_centre = metal::float2(0.0);
        Pile _e119 = out_4;
        return _e119;
    }
    metal::float2 _e121 = face;
    float _e122 = weight;
    out_4.face = _e121 / metal::float2(_e122);
    metal::float2 _e126 = to_centre;
    float _e127 = weight;
    out_4.to_centre = _e126 / metal::float2(_e127);
    Pile _e130 = out_4;
    return _e130;
}

Pile cloud_domes(
    metal::float2 r_5,
    int period_8,
    constant Cloud& cloud
) {
    Pile out_5 = {};
    Pile _e2 = dome_octave(r_5, period_8, cloud);
    Pile _e14 = dome_octave((r_5 * DOME_LACUNARITY) + metal::float2(17.3, 5.9), naga_f2i32(metal::rint(DOME_LACUNARITY * static_cast<float>(period_8))), cloud);
    out_5.face = (_e2.face + (0.46199998 * _e14.face)) / metal::float2(1.22);
    out_5.to_centre = _e2.to_centre;
    Pile _e27 = out_5;
    return _e27;
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
        float _e39 = cloud.wash_fuzz;
        float _e42 = cloud.wash_lobe;
        WashField _e43 = wash_field(wash_cell, period_9, _e39, _e42);
        out.a = metal::float4(_e43.coarse.offset, 0.0, 0.0);
        out.b = metal::float4(_e43.fine.offset, 0.0, _e43.cover);
    } else {
        Pile _e56 = cloud_domes(mosaic_cell, period_9, cloud);
        out.a = metal::float4(_e56.face, _e56.to_centre);
    }
    TileBake _e61 = out;
    const auto _tmp = _e61;
    return fs_cloud_tileOutput { _tmp.a, _tmp.b };
}
