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
    uint cloud_style;
    float wash_size;
    float wash_fuzz;
    float wash_lobe;
    float wash_refract;
    float wash_layers;
    uint tile_cells;
    uint pitch_vertical;
    metal::uint2 _pad;
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

bool softened(
    constant Cloud& cloud
) {
    metal::float2 _e2 = cloud.step;
    return metal::any(_e2 != metal::float2(0.0));
}

float style_level(
    float level,
    constant Cloud& cloud
) {
    float _e3 = cloud.contour_strength;
    if (_e3 <= 0.0) {
        return level;
    }
    float _e11 = cloud.contours;
    float x = metal::clamp(level, 0.0, 1.0) * _e11;
    float _e15 = cloud.contour_softness;
    float _e16 = metal::fwidth(x);
    float edge = metal::min(0.5, metal::max(_e15, _e16 * 0.5));
    float _e32 = cloud.contours;
    float terraces = (metal::floor(x) + metal::smoothstep(0.5 - edge, 0.5 + edge, metal::fract(x))) / _e32;
    float _e36 = cloud.contour_strength;
    float _e43 = metal::fwidth(x);
    float strength = ((0.9 * _e36) * metal::smoothstep(0.0, 1.0, x)) * (1.0 - metal::smoothstep(0.5, 1.5, _e43));
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
    float x_1 = metal::max((metal::clamp(level_1, 0.0, 1.0) * static_cast<float>(levels)) - 0.5, 0.0);
    uint i_2 = naga_f2u32(metal::clamp(metal::floor(x_1), 0.0, static_cast<float>(levels - 1u)));
    uint clamped_lod_e24 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e24 = lut.read(metal::min(metal::uint2(metal::uint2(i_2, 0u)), metal::uint2(lut.get_width(clamped_lod_e24), lut.get_height(clamped_lod_e24)) - 1), clamped_lod_e24);
    metal::float3 a = _e24.xyz;
    uint clamped_lod_e35 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e35 = lut.read(metal::min(metal::uint2(metal::uint2(metal::min(i_2 + 1u, levels - 1u), 0u)), metal::uint2(lut.get_width(clamped_lod_e35), lut.get_height(clamped_lod_e35)) - 1), clamped_lod_e35);
    metal::float3 b = _e35.xyz;
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

metal::float2 watercolor_tile_uv_for(
    metal::float2 r,
    float period_2,
    uint pitch_vertical
) {
    metal::float2 semantic = (pitch_vertical == 1u) ? r : metal::float2(r.y, r.x);
    return metal::float2((CLOUD_TILE_ROT_COS * semantic.x) + (CLOUD_TILE_ROT_SIN * semantic.y), (-0.6 * semantic.x) + (CLOUD_TILE_ROT_COS * semantic.y)) / metal::float2(period_2);
}

metal::float2 watercolor_tile_uv(
    metal::float2 r_1,
    constant Cloud& cloud
) {
    uint _e3 = cloud.tile_cells;
    uint _e7 = cloud.pitch_vertical;
    metal::float2 _e8 = watercolor_tile_uv_for(r_1, static_cast<float>(_e3), _e7);
    return _e8;
}

metal::float2 rotate_watercolor_tile_vector_for(
    metal::float2 v,
    uint pitch_vertical_1
) {
    metal::float2 semantic_1 = (pitch_vertical_1 == 1u) ? v : metal::float2(v.y, v.x);
    metal::float2 turned = metal::float2((CLOUD_TILE_ROT_COS * semantic_1.x) - (CLOUD_TILE_ROT_SIN * semantic_1.y), (CLOUD_TILE_ROT_SIN * semantic_1.x) + (CLOUD_TILE_ROT_COS * semantic_1.y));
    return (pitch_vertical_1 == 1u) ? turned : metal::float2(turned.y, turned.x);
}

metal::float2 rotate_watercolor_tile_vector(
    metal::float2 v_1,
    constant Cloud& cloud
) {
    uint _e3 = cloud.pitch_vertical;
    metal::float2 _e4 = rotate_watercolor_tile_vector_for(v_1, _e3);
    return _e4;
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
    metal::float2 r_2,
    int period_3,
    constant Cloud& cloud
) {
    float weight = 0.0;
    metal::float2 face = metal::float2(0.0);
    metal::float2 to_centre = metal::float2(0.0);
    int j = -1;
    int i = {};
    float gain = {};
    Pile out = {};
    metal::float2 base = metal::floor(r_2);
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
                    metal::int2 _e26 = wrap_cell(cell_5, period_3);
                    metal::float4 _e27 = cloud_hash4_(_e26);
                    int _e28 = i;
                    int _e30 = j;
                    metal::float2 centre = ((base + metal::float2(static_cast<float>(_e28), static_cast<float>(_e30))) + metal::float2(0.5)) + ((_e27.xy - metal::float2(0.5)) * DOME_JITTER);
                    float _e51 = cloud.scale_variety;
                    float radius = metal::mix(DOME_RADIUS, metal::mix(DOME_RADIUS_MIN, DOME_RADIUS_MAX, _e27.z), _e51);
                    metal::float2 d = (r_2 - centre) / metal::float2(radius);
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
                    to_centre = _e99 + (w * (centre - r_2));
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
    metal::float2 r_3,
    int period_4,
    constant Cloud& cloud
) {
    Pile out_1 = {};
    Pile _e2 = dome_octave(r_3, period_4, cloud);
    Pile _e14 = dome_octave((r_3 * DOME_LACUNARITY) + metal::float2(17.3, 5.9), naga_f2i32(metal::rint(DOME_LACUNARITY * static_cast<float>(period_4))), cloud);
    out_1.face = (_e2.face + (0.46199998 * _e14.face)) / metal::float2(1.22);
    out_1.to_centre = _e2.to_centre;
    Pile _e27 = out_1;
    return _e27;
}

float cloud_light(
    metal::float2 pt,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    metal::float2 _e5 = cloud.size;
    metal::float4 _e8 = close_light.sample(cloud_sampler, pt / _e5, metal::level(0.0));
    return _e8.x;
}

float scale_tone(
    metal::float2 pt_1,
    metal::texture2d<float, metal::access::sample> close_light,
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
    metal::float2 r_9 = q_1 * scale_units;
    uint _e35 = cloud.tile_cells;
    if (_e35 > 0u) {
        uint _e42 = cloud.tile_cells;
        metal::float4 tile = cloud_tile_a.sample(tile_sampler, r_9 / metal::float2(static_cast<float>(_e42)), metal::level(0.0));
        pile.face = tile.xy;
        pile.to_centre = tile.zw;
    } else {
        Pile _e53 = cloud_domes(r_9, 0, cloud);
        pile = _e53;
    }
    metal::float2 face_1 = pile.face;
    float _e58 = cloud.scale_refract;
    float bend = metal::max(_e58, 0.0) * scale_points;
    float _e64 = cloud.scale_refract;
    float gather = metal::max(-(_e64), 0.0) * scale_points;
    metal::float2 _e72 = pile.to_centre;
    metal::float2 lookup = (-(face_1) * bend) + (_e72 * gather);
    float _e76 = cloud_light(pt_1 + lookup, close_light, cloud_sampler, cloud);
    return _e76;
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
    int period_5
) {
    metal::float2 b_1 = metal::floor(p);
    metal::float2 f_1 = p - b_1;
    metal::float2 t = (f_1 * f_1) * (metal::float2(3.0) - (2.0 * f_1));
    metal::int2 i_3 = naga_f2i32(b_1);
    metal::int2 _e13 = wrap_cell(i_3, period_5);
    metal::float3 _e14 = wash_hash(_e13, salt_1);
    float n00_ = _e14.x;
    metal::int2 _e20 = wrap_cell(as_type<metal::int2>(as_type<metal::uint2>(i_3) + as_type<metal::uint2>(metal::int2(1, 0))), period_5);
    metal::float3 _e21 = wash_hash(_e20, salt_1);
    float n10_ = _e21.x;
    metal::int2 _e27 = wrap_cell(as_type<metal::int2>(as_type<metal::uint2>(i_3) + as_type<metal::uint2>(metal::int2(0, 1))), period_5);
    metal::float3 _e28 = wash_hash(_e27, salt_1);
    float n01_ = _e28.x;
    metal::int2 _e34 = wrap_cell(as_type<metal::int2>(as_type<metal::uint2>(i_3) + as_type<metal::uint2>(metal::int2(1, 1))), period_5);
    metal::float3 _e35 = wash_hash(_e34, salt_1);
    float n11_ = _e35.x;
    return metal::mix(metal::mix(n00_, n10_, t.x), metal::mix(n01_, n11_, t.x), t.y);
}

float wash_fbm(
    metal::float2 p_1,
    uint salt_2,
    int period_6
) {
    float lacunarity = (period_6 > 0) ? WASH_FBM_FINE_TILED : WASH_FBM_FINE;
    float _e8 = wash_noise(p_1, salt_2, period_6);
    float _e18 = wash_noise((p_1 * lacunarity) + metal::float2(13.1, -7.3), salt_2 + 31u, as_type<int>(as_type<uint>(period_6) * as_type<uint>(2)));
    return (_e8 + (0.5 * _e18)) / 1.5;
}

Glob wash_glob(
    metal::int2 cell_4,
    uint salt_3,
    metal::float2 r_4,
    float occupancy,
    int period_7
) {
    Glob out_2 = {};
    metal::int2 _e5 = wrap_cell(cell_4, period_7);
    metal::float3 _e8 = wash_hash(_e5, salt_3 + 77u);
    out_2.order = _e8.x;
    if (_e8.y > occupancy) {
        out_2.centre = r_4;
        out_2.edge = 1000000000.0;
        Glob _e17 = out_2;
        return _e17;
    }
    metal::float3 _e18 = wash_hash(_e5, salt_3);
    metal::float2 centre_1 = (static_cast<metal::float2>(cell_4) + metal::float2(0.5)) + ((_e18.xy - metal::float2(0.5)) * WASH_JITTER);
    float radius_1 = metal::mix(WASH_RADIUS_MIN, WASH_RADIUS_MAX, _e18.z);
    out_2.centre = centre_1;
    out_2.edge = metal::length(r_4 - centre_1) / radius_1;
    Glob _e39 = out_2;
    return _e39;
}

Wash wash_scan(
    metal::float2 r_5,
    uint salt_4,
    float occupancy_1,
    int period_8
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
    out_3.centre = r_5;
    out_3.under = r_5;
    out_3.front = r_5;
    out_3.near = -1000000000.0;
    out_3.edge = 1.0;
    out_3.cover = 0.0;
    front_a = r_5;
    front_b = r_5;
    metal::int2 base_1 = naga_f2i32(metal::floor(r_5));
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
                    Glob _e44 = wash_glob(as_type<metal::int2>(as_type<metal::uint2>(base_1) + as_type<metal::uint2>(metal::int2(_e40, _e41))), salt_4, r_5, occupancy_1, period_8);
                    float prox = 1.0 - _e44.edge;
                    float _e50 = out_3.cover;
                    out_3.cover = metal::max(_e50, metal::clamp(prox / 0.05, 0.0, 1.0));
                    if (_e44.edge < 1.0) {
                        float _e61 = best;
                        if (_e44.order > _e61) {
                            float _e63 = best;
                            second = _e63;
                            metal::float2 _e66 = out_3.centre;
                            out_3.under = _e66;
                            best = _e44.order;
                            out_3.centre = _e44.centre;
                            out_3.edge = _e44.edge;
                        } else {
                            float _e73 = second;
                            if (_e44.order > _e73) {
                                second = _e44.order;
                                out_3.under = _e44.centre;
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
        out_3.near = _e99;
        metal::float2 _e101 = front_b;
        out_3.front = _e101;
    }
    float _e102 = order_a;
    float _e103 = best;
    if (_e102 > _e103) {
        float _e106 = near_a;
        out_3.near = _e106;
        metal::float2 _e108 = front_a;
        out_3.front = _e108;
    }
    Wash _e109 = out_3;
    return _e109;
}

Wet wash_wet(
    Wash f,
    metal::float2 r_6,
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
    return Wet {_e64 - r_6};
}

float wash_level(
    Wet wet,
    float pane_per_cell,
    metal::float2 pt_2,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    float _e7 = cloud.wash_refract;
    float _e10 = cloud_light(pt_2 + ((wet.offset * pane_per_cell) * _e7), close_light, cloud_sampler, cloud);
    return _e10;
}

WashField wash_field(
    metal::float2 r_7,
    int period_9,
    bool want_fine,
    constant Cloud& cloud
) {
    metal::float2 warped = {};
    WashField out_4 = {};
    warped = r_7;
    float _e6 = cloud.wash_lobe;
    if (_e6 > 0.0) {
        float _e12 = cloud.wash_lobe;
        float amp = WASH_WARP * _e12;
        int warp_period = naga_f2i32(metal::rint(WASH_WARP_SCALE * static_cast<float>(period_9)));
        metal::float2 _e19 = warped;
        float _e25 = wash_fbm(r_7 * WASH_WARP_SCALE, 71u, warp_period);
        float _e35 = wash_fbm((r_7 * WASH_WARP_SCALE) + metal::float2(37.0, -19.0), 73u, warp_period);
        warped = _e19 + ((amp * 2.0) * metal::float2(_e25 - 0.5, _e35 - 0.5));
    }
    metal::float2 _e43 = warped;
    Wash _e46 = wash_scan(_e43, 1u, 1.0, period_9);
    metal::float2 _e47 = warped;
    Wet _e48 = wash_wet(_e46, _e47, cloud);
    out_4.coarse = _e48;
    out_4.fine = Wet {metal::float2(0.0)};
    out_4.cover = 0.0;
    if (want_fine) {
        metal::float2 _e55 = warped;
        metal::float2 fine_r = (_e55 * WASH_LACUNARITY) + metal::float2(17.3, 5.9);
        Wash _e69 = wash_scan(fine_r, 2u, WASH_FINE_OCCUPANCY, naga_f2i32(metal::rint(WASH_LACUNARITY * static_cast<float>(period_9))));
        Wet _e71 = wash_wet(_e69, fine_r, cloud);
        out_4.fine = _e71;
        out_4.cover = _e69.cover;
    }
    WashField _e74 = out_4;
    return _e74;
}

WashField wash_tile_field(
    metal::float2 r_8,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    WashField out_5 = {};
    metal::float2 _e1 = watercolor_tile_uv(r_8, cloud);
    metal::float4 a_1 = cloud_tile_a.sample(tile_sampler, _e1, metal::level(0.0));
    metal::float2 _e9 = rotate_watercolor_tile_vector(a_1.xy, cloud);
    out_5.coarse = Wet {_e9};
    out_5.fine = Wet {metal::float2(0.0)};
    out_5.cover = 0.0;
    float _e19 = cloud.wash_layers;
    if (_e19 > 0.0) {
        metal::float4 b_2 = cloud_tile_b.sample(tile_sampler, _e1, metal::level(0.0));
        metal::float2 _e28 = rotate_watercolor_tile_vector(b_2.xy, cloud);
        out_5.fine = Wet {_e28};
        out_5.cover = b_2.w;
    }
    WashField _e32 = out_5;
    return _e32;
}

float wash_cloud_tone(
    metal::float2 pt_3,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    WashField field = {};
    float level_2 = {};
    metal::float2 _e3 = cloud.size;
    float _e10 = cloud.size.y;
    metal::float2 _e17 = cloud.drift;
    metal::float2 q_2 = (((pt_3 - (_e3 * 0.5)) / metal::float2(_e10)) * CLOUD_UNITS) + _e17;
    float _e22 = cloud.wash_size;
    float cells = WASH_CELLS / _e22;
    float _e27 = cloud.size.y;
    float pane_per_cell_1 = (_e27 / CLOUD_UNITS) / cells;
    metal::float2 r_10 = q_2 * cells;
    uint _e35 = cloud.tile_cells;
    if (_e35 > 0u) {
        WashField _e38 = wash_tile_field(r_10, cloud, cloud_tile_a, cloud_tile_b, tile_sampler);
        field = _e38;
    } else {
        float _e42 = cloud.wash_layers;
        WashField _e45 = wash_field(r_10, 0, _e42 > 0.0, cloud);
        field = _e45;
    }
    Wet _e47 = field.coarse;
    float _e48 = wash_level(_e47, pane_per_cell_1, pt_3, close_light, cloud_sampler, cloud);
    level_2 = _e48;
    float _e52 = cloud.wash_layers;
    if (_e52 > 0.0) {
        Wet _e56 = field.fine;
        float _e59 = wash_level(_e56, pane_per_cell_1 / WASH_LACUNARITY, pt_3, close_light, cloud_sampler, cloud);
        float _e62 = cloud.wash_layers;
        float _e64 = field.cover;
        float over = _e62 * _e64;
        float _e66 = level_2;
        level_2 = metal::mix(_e66, _e59, over);
    }
    float _e68 = level_2;
    return _e68;
}

float cloud_tone_at(
    metal::float2 pt_4,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    uint _e3 = cloud.cloud_style;
    if (_e3 == 1u) {
        float _e6 = wash_cloud_tone(pt_4, close_light, cloud_sampler, cloud, cloud_tile_a, cloud_tile_b, tile_sampler);
        return _e6;
    }
    float _e7 = scale_tone(pt_4, close_light, cloud_sampler, cloud, cloud_tile_a, tile_sampler);
    return _e7;
}

metal::float4 clouded(
    float level_3,
    metal::float2 position_1,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tone,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    float tone = {};
    float _e4 = cloud.cloud_depth;
    if (_e4 <= 0.0) {
        metal::float4 _e7 = density_color(level_3, lut, cloud);
        return _e7;
    }
    float _e10 = cloud.ppp;
    metal::float2 _e15 = cloud.origin;
    metal::float2 pt_5 = (position_1 / metal::float2(_e10)) - _e15;
    uint _e20 = cloud.tone_baked;
    if (_e20 == 1u) {
        metal::float2 _e27 = cloud.size;
        metal::float4 _e30 = cloud_tone.sample(cloud_sampler, pt_5 / _e27, metal::level(0.0));
        tone = _e30.x;
    } else {
        float _e32 = cloud_tone_at(pt_5, close_light, cloud_sampler, cloud, cloud_tile_a, cloud_tile_b, tile_sampler);
        tone = _e32;
    }
    float _e33 = tone;
    float _e36 = cloud.cloud_depth;
    metal::float4 _e38 = density_color(metal::mix(level_3, _e33, _e36), lut, cloud);
    return _e38;
}

metal::float4 backdrop_color(
    metal::float2 position_2,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tone,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    float level_4 = 0.0;
    bool _e3 = softened(cloud);
    if (_e3) {
        float _e4 = baked_density(position_2, close_light, cloud_sampler, cloud);
        level_4 = _e4;
    }
    float _e5 = level_4;
    metal::float4 _e6 = clouded(_e5, position_2, lut, close_light, cloud_sampler, cloud, cloud_tone, cloud_tile_a, cloud_tile_b, tile_sampler);
    return _e6;
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
, metal::sampler cloud_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(2)]]
, metal::texture2d<float, metal::access::sample> cloud_tone [[texture(3)]]
, metal::texture2d<float, metal::access::sample> cloud_tile_a [[texture(4)]]
, metal::texture2d<float, metal::access::sample> cloud_tile_b [[texture(5)]]
, metal::sampler tile_sampler [[sampler(1)]]
) {
    const VertexOut in = { position_3, varyings.slab, varyings.t };
    metal::float4 _e3 = backdrop_color(in.position.xy, lut, close_light, cloud_sampler, cloud, cloud_tone, cloud_tile_a, cloud_tile_b, tile_sampler);
    return fs_cloud_backdrop_gammaOutput { _e3 };
}
