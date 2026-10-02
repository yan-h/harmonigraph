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
    bool has_under;
    char _pad3[7];
    metal::float2 front;
    float near;
    float edge;
    float cover;
    char _pad7[4];
};
struct Wet {
    metal::float2 offset;
    float brightness;
    float gap;
};
struct WashField {
    Wet coarse;
    Wet fine;
    float cover;
    char _pad3[4];
};
struct StarHaloSample {
    metal::float2 size;
    uint group;
    uint layer;
};
struct StarSlice {
    metal::float2 offset;
    float cell;
    float radius;
    float solid;
    float ramp;
    float bend;
    int base;
    metal::int2 origin;
    metal::int2 grid;
    float width;
    float inner;
    float gain;
    uint gather;
};
struct type_8 {
    StarSlice inner[5];
};
struct type_9 {
    StarHaloSample inner[5];
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
    float star_size_variation;
    uint star_pad0_;
    uint star_pad1_;
    metal::float4 star_far;
    metal::float4 star_near;
    type_8 star_slices;
    uint memory_enabled;
    uint memory_valid;
    float pickup_alpha;
    float release_alpha;
    metal::int2 memory_shift;
    metal::float2 memory_fraction;
    float previous_life;
    float wash_randomness;
    metal::float2 memory_extent;
    type_8 previous_slices;
    type_9 star_halo_samples;
    metal::float4 velvet;
    metal::float4 velvet_form;
    metal::float4 wash_pigment;
};
struct TileVertex {
    metal::float4 position;
    metal::float2 fraction;
    uint layer;
    char _pad3[4];
};
struct TileBake {
    metal::float4 a;
    metal::float4 b;
    metal::float4 c;
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
constant int STAR_ATLAS_WIDTH = 2048;
constant uint STAR_ATLAS_SHIFT = 11u;
constant uint STAR_SLICES = 5u;
constant float STAR_PANE = 540.0;
constant int STAR_HASH_PERIOD = 65536;
constant uint STAR_LIFE_PERIOD = 4096u;
constant float STAR_FADE = 0.2;
constant float STAR_LIFT = 0.18;
constant float STAR_INNER_FADE = 0.7;
constant uint STAR_FAR_LAYERS = 3u;
constant float CLOUD_UNITS = 10.0;
constant float WASH_POOL = 0.44;
constant float WASH_PIG_DEPTH = 0.35;
constant float WASH_BLOOM = 2.7;

metal::int2 naga_mod(metal::int2 lhs, metal::int2 rhs) {
    metal::int2 divisor = metal::select(rhs, 1, (lhs == (-2147483647 - 1) & rhs == -1) | (rhs == 0));
    return lhs - (lhs / divisor) * divisor;
}

metal::int2 wrap_cell(
    metal::int2 cell,
    int period
) {
    return naga_mod(as_type<metal::int2>(as_type<metal::uint2>(naga_mod(cell, metal::int2(period))) + as_type<metal::uint2>(metal::int2(period))), metal::int2(period));
}

metal::float3 wash_hash(
    metal::int2 cell_1,
    uint salt
) {
    uint n = {};
    n = (as_type<uint>(cell_1.x) * 2654435769u) ^ (as_type<uint>(cell_1.y) * 2246822507u);
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
    int period_1
) {
    metal::float2 b = metal::floor(p);
    metal::float2 f_1 = p - b;
    metal::float2 t = (f_1 * f_1) * (metal::float2(3.0) - (2.0 * f_1));
    metal::int2 i_1 = naga_f2i32(b);
    metal::int2 _e13 = wrap_cell(i_1, period_1);
    metal::float3 _e14 = wash_hash(_e13, salt_1);
    float n00_ = _e14.x;
    metal::int2 _e20 = wrap_cell(as_type<metal::int2>(as_type<metal::uint2>(i_1) + as_type<metal::uint2>(metal::int2(1, 0))), period_1);
    metal::float3 _e21 = wash_hash(_e20, salt_1);
    float n10_ = _e21.x;
    metal::int2 _e27 = wrap_cell(as_type<metal::int2>(as_type<metal::uint2>(i_1) + as_type<metal::uint2>(metal::int2(0, 1))), period_1);
    metal::float3 _e28 = wash_hash(_e27, salt_1);
    float n01_ = _e28.x;
    metal::int2 _e34 = wrap_cell(as_type<metal::int2>(as_type<metal::uint2>(i_1) + as_type<metal::uint2>(metal::int2(1, 1))), period_1);
    metal::float3 _e35 = wash_hash(_e34, salt_1);
    float n11_ = _e35.x;
    return metal::mix(metal::mix(n00_, n10_, t.x), metal::mix(n01_, n11_, t.x), t.y);
}

float wash_fbm(
    metal::float2 p_1,
    uint salt_2,
    int period_2
) {
    float _e3 = wash_noise(p_1, salt_2, period_2);
    float _e14 = wash_noise((p_1 * WASH_FBM_FINE) + metal::float2(13.1, -7.3), salt_2 + 31u, as_type<int>(as_type<uint>(period_2) * as_type<uint>(2)));
    return (_e3 + (0.5 * _e14)) / 1.5;
}

Glob wash_glob(
    metal::int2 cell_2,
    uint salt_3,
    metal::float2 r,
    float occupancy,
    int period_3
) {
    Glob out_1 = {};
    metal::int2 _e5 = wrap_cell(cell_2, period_3);
    metal::float3 _e8 = wash_hash(_e5, salt_3 + 77u);
    out_1.order = _e8.x;
    if (_e8.y > occupancy) {
        out_1.centre = r;
        out_1.edge = 1000000000.0;
        Glob _e17 = out_1;
        return _e17;
    }
    metal::float3 _e18 = wash_hash(_e5, salt_3);
    metal::float2 centre_1 = (static_cast<metal::float2>(cell_2) + metal::float2(0.5)) + ((_e18.xy - metal::float2(0.5)) * WASH_JITTER);
    float radius = metal::mix(WASH_RADIUS_MIN, WASH_RADIUS_MAX, _e18.z);
    out_1.centre = centre_1;
    out_1.edge = metal::length(r - centre_1) / radius;
    Glob _e39 = out_1;
    return _e39;
}

Wash wash_scan(
    metal::float2 r_1,
    uint salt_4,
    float occupancy_1,
    int period_4
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
                    Glob _e44 = wash_glob(as_type<metal::int2>(as_type<metal::uint2>(base) + as_type<metal::uint2>(metal::int2(_e40, _e41))), salt_4, r_1, occupancy_1, period_4);
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
    float _e110 = second;
    out_2.has_under = _e110 >= 0.0;
    Wash _e113 = out_2;
    return _e113;
}

float wash_brightness(
    metal::float2 centre,
    uint salt_5,
    int period_5
) {
    metal::int2 _e5 = wrap_cell(naga_f2i32(metal::floor(centre)), period_5);
    metal::float3 _e8 = wash_hash(_e5, salt_5 + 197u);
    return (2.0 * _e8.x) - 1.0;
}

Wet wash_wet(
    Wash f,
    metal::float2 r_2,
    float fuzz,
    uint salt_6,
    int period_6
) {
    metal::float2 look = {};
    float fa = {};
    float bl = {};
    float feather = 0.1 + (0.8 * fuzz);
    float bleed = 0.12 + (0.78 * fuzz);
    look = f.centre;
    fa = metal::clamp((f.edge - (1.0 - feather)) / feather, 0.0, 1.0);
    float _e24 = fa;
    float _e25 = fa;
    float _e27 = fa;
    fa = ((_e24 * _e25) * (3.0 - (2.0 * _e27))) * 0.5;
    metal::float2 _e35 = look;
    float _e37 = fa;
    look = metal::mix(_e35, f.under, _e37);
    bl = metal::clamp((f.near + bleed) / bleed, 0.0, 1.0);
    float _e46 = bl;
    float _e47 = bl;
    float _e49 = bl;
    bl = ((_e46 * _e47) * (3.0 - (2.0 * _e49))) * 0.5;
    metal::float2 _e57 = look;
    float _e59 = bl;
    look = metal::mix(_e57, f.front, _e59);
    float _e62 = wash_brightness(f.centre, salt_6, period_6);
    float centre_2 = (f.cover > 0.0) ? _e62 : 0.0;
    float _e69 = wash_brightness(f.under, salt_6, period_6);
    float under = f.has_under ? _e69 : 0.0;
    float _e73 = fa;
    float _e76 = wash_brightness(f.front, salt_6, period_6);
    float _e77 = bl;
    float brightness = metal::mix(metal::mix(centre_2, under, _e73), _e76, _e77);
    metal::float2 _e79 = look;
    return Wet {_e79 - r_2, brightness, metal::clamp(-(f.near), 0.0, 1.0)};
}

int naga_f2i32(float value) {
    return static_cast<int>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

WashField wash_field(
    metal::float2 r_3,
    int period_7,
    float fuzz_1,
    float lobe
) {
    metal::float2 warped = {};
    WashField out_3 = {};
    warped = r_3;
    if (lobe > 0.0) {
        float amp = WASH_WARP * lobe;
        int warp_period = naga_f2i32(metal::rint(WASH_WARP_SCALE * static_cast<float>(period_7)));
        metal::float2 _e14 = warped;
        float _e20 = wash_fbm(r_3 * WASH_WARP_SCALE, 71u, warp_period);
        float _e30 = wash_fbm((r_3 * WASH_WARP_SCALE) + metal::float2(37.0, -19.0), 73u, warp_period);
        warped = _e14 + ((amp * 2.0) * metal::float2(_e20 - 0.5, _e30 - 0.5));
    }
    metal::float2 _e38 = warped;
    Wash _e41 = wash_scan(_e38, 1u, 1.0, period_7);
    metal::float2 _e42 = warped;
    Wet _e44 = wash_wet(_e41, _e42, fuzz_1, 1u, period_7);
    out_3.coarse = _e44;
    metal::float2 _e45 = warped;
    metal::float2 fine_r = (_e45 * WASH_LACUNARITY) + metal::float2(17.3, 5.9);
    Wash _e59 = wash_scan(fine_r, 2u, WASH_FINE_OCCUPANCY, naga_f2i32(metal::rint(WASH_LACUNARITY * static_cast<float>(period_7))));
    Wet _e67 = wash_wet(_e59, fine_r, fuzz_1, 2u, naga_f2i32(metal::rint(WASH_LACUNARITY * static_cast<float>(period_7))));
    out_3.fine = _e67;
    out_3.cover = _e59.cover;
    WashField _e70 = out_3;
    return _e70;
}

struct fs_cloud_tileInput {
    metal::float2 fraction [[user(loc0), center_perspective]];
    uint layer [[user(loc1), flat]];
};
struct fs_cloud_tileOutput {
    metal::float4 a [[color(0)]];
    metal::float4 b [[color(1)]];
    metal::float4 c [[color(2)]];
};
fragment fs_cloud_tileOutput fs_cloud_tile(
  fs_cloud_tileInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Cloud& cloud [[buffer(2)]]
) {
    const TileVertex in = { position, varyings.fraction, varyings.layer };
    TileBake out = {};
    uint _e3 = cloud.tile_cells;
    int period_8 = static_cast<int>(_e3);
    uint _e7 = cloud.tile_cells;
    float period_f = static_cast<float>(_e7);
    float time = in.fraction.x * period_f;
    float pitch = in.fraction.y * period_f;
    uint _e19 = cloud.pitch_vertical;
    metal::float2 wash_cell = (_e19 == 1u) ? metal::float2(time, pitch) : metal::float2(pitch, time);
    float _e25 = cloud.wash_fuzz;
    float _e28 = cloud.wash_lobe;
    WashField _e29 = wash_field(wash_cell, period_8, _e25, _e28);
    out.a = metal::float4(_e29.coarse.offset, _e29.coarse.brightness, 0.0);
    out.b = metal::float4(_e29.fine.offset, _e29.fine.brightness, _e29.cover);
    out.c = metal::float4(_e29.coarse.gap, _e29.fine.gap, 0.0, 0.0);
    TileBake _e53 = out;
    const auto _tmp = _e53;
    return fs_cloud_tileOutput { _tmp.a, _tmp.b, _tmp.c };
}
