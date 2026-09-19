// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint size1;
};

struct Locals {
    metal::float2 origin_points;
    metal::float2 viewport_points;
    float min_midi;
    float span;
    float spectrum_min_midi;
    float bins_per_semitone;
    float level0_;
    float level_per_step;
    float level_per_midi;
    uint rows;
    uint bins;
    uint stride;
    uint capacity;
    uint first_slot;
    uint run_slabs;
    uint _pad0_;
    uint _pad1_;
    uint _pad2_;
};
typedef uint type_3[1];
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
    float scale_size;
    float scale_variety;
    float scale_refract;
    float scale_relief;
    float scale_shade_floor;
    float scale_facet;
    float scale_rock;
    uint cloud_style;
    float wash_size;
    float wash_variety;
    float wash_fuzz;
    float wash_ragged;
    float wash_lobe;
    float wash_refract;
    float wash_pool;
    float wash_grain;
    float wash_layers;
    float wash_soften;
    float wash_wander;
};
struct Pile {
    metal::float2 face;
    metal::float2 to_centre;
    metal::float2 rock;
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
    float tau;
};
constant float DOME_RADIUS = 1.15;
constant float DOME_JITTER = 0.3;
constant float DOME_RADIUS_MIN = 0.95;
constant float DOME_RADIUS_MAX = 1.32;
constant float DOME_UNION = 9.0;
constant float DOME_FACE = 1.5122874;
constant float ROCK_TILT = 0.3;
constant float SUN_LEAN = 1.0;
constant float SUN_KNEE = 0.03;
constant float DOME_LACUNARITY = 2.1;
constant float DOME_FINE_GAIN = 0.22;
constant float CLOUD_SHADE = 0.64;
constant int WASH_RING = 2;
constant float WASH_JITTER = 0.4;
constant float WASH_RAGGED = 0.3;
constant float WASH_RADIUS = 1.18;
constant float WASH_RADIUS_MIN = 1.02;
constant float WASH_RADIUS_MAX = 1.66;
constant float WASH_CELLS = 5.25;
constant float WASH_LACUNARITY = 2.1;
constant float WASH_FINE_OCCUPANCY = 0.2;
constant float WASH_WARP = 0.45;
constant float WASH_WARP_SCALE = 0.9;
constant float WASH_RAGGED_SCALE = 2.8;
constant float WASH_POOL = 0.44;
constant float WASH_GRAIN = 0.1;
constant float WASH_POOL_WIDTH = 0.55;
constant float WASH_SURF = 0.07;
constant float WASH_PIG_DEPTH = 0.35;
constant float WASH_TONE_FLOOR = 0.05;
constant float WASH_PIVOT = 0.45;
constant float WASH_LIFT_A = 1.15;
constant float WASH_LIFT_B = 0.16;

uint stored(
    uint slot,
    uint bucket,
    constant Locals& locals,
    device type_3 const& grid,
    constant _mslBufferSizes& _buffer_sizes
) {
    uint _e4 = locals.stride;
    uint i_2 = (slot * _e4) + bucket;
    uint _e11 = grid[metal::min(unsigned(i_2 >> 2u), (_buffer_sizes.size1 - 0 - 4) / 4)];
    return (_e11 >> ((i_2 & 3u) * 8u)) & 255u;
}

float bucket_x(
    float t,
    constant Locals& locals
) {
    float _e3 = locals.min_midi;
    float _e6 = locals.span;
    float midi = _e3 + (t * _e6);
    float _e11 = locals.spectrum_min_midi;
    float _e15 = locals.bins_per_semitone;
    return (midi - _e11) * _e15;
}

float density_encode(
    float level
) {
    return level * (0.1 + (0.9 * level));
}

float density_decode(
    float value
) {
    float y = metal::max(value, 0.0);
    return (2.0 * y) / (0.1 + metal::sqrt(0.01 + (3.6 * y)));
}

float bucket_level(
    uint slot_1,
    uint b,
    bool density,
    constant Locals& locals,
    device type_3 const& grid,
    constant _mslBufferSizes& _buffer_sizes
) {
    float _e5 = locals.spectrum_min_midi;
    float _e11 = locals.bins_per_semitone;
    float midi_1 = _e5 + ((static_cast<float>(b) + 0.5) / _e11);
    uint _e14 = stored(slot_1, b, locals, grid, _buffer_sizes);
    float v = static_cast<float>(_e14);
    float _e18 = locals.level0_;
    float _e21 = locals.level_per_step;
    float _e26 = locals.level_per_midi;
    float level_4 = (_e18 + (_e21 * v)) + (_e26 * midi_1);
    float mapped = metal::clamp(level_4, 0.0, 1.0);
    if (density) {
        float _e32 = density_encode(mapped);
        return _e32;
    }
    return mapped;
}

uint naga_f2u32(float value) {
    return static_cast<uint>(metal::clamp(value, 0.0, 4294967000.0));
}

float read_level(
    uint slot_2,
    float t_1,
    bool density_1,
    constant Locals& locals,
    device type_3 const& grid,
    constant _mslBufferSizes& _buffer_sizes
) {
    float sum = 0.0;
    float total = 0.0;
    uint b_1 = {};
    uint _e5 = locals.rows;
    float half_ = 0.5 / static_cast<float>(_e5);
    float _e10 = bucket_x(t_1 - half_, locals);
    float _e12 = bucket_x(t_1 + half_, locals);
    uint _e15 = locals.bins;
    float top = static_cast<float>(_e15) - 1.0;
    uint idx = naga_f2u32(metal::clamp(metal::floor(_e10), 0.0, top));
    uint last = naga_f2u32(metal::clamp(metal::floor(_e12), 0.0, top));
    if (last > idx) {
        uint _e30 = locals.bins;
        float lo = metal::clamp(_e10, 0.0, static_cast<float>(_e30));
        uint _e36 = locals.bins;
        float hi = metal::clamp(_e12, 0.0, static_cast<float>(_e36));
        b_1 = idx;
        uint2 loop_bound = uint2(4294967295u);
        bool loop_init = true;
        while(true) {
            if (metal::all(loop_bound == uint2(0u))) { break; }
            loop_bound -= uint2(loop_bound.y == 0u, 1u);
            if (!loop_init) {
                uint _e65 = b_1;
                b_1 = _e65 + 1u;
            }
            loop_init = false;
            uint _e45 = b_1;
            if (_e45 <= last) {
            } else {
                break;
            }
            {
                uint _e47 = b_1;
                uint _e52 = b_1;
                float w = metal::max(metal::min(hi, static_cast<float>(_e47) + 1.0) - metal::max(lo, static_cast<float>(_e52)), 0.0);
                float _e58 = sum;
                uint _e59 = b_1;
                float _e60 = bucket_level(slot_2, _e59, density_1, locals, grid, _buffer_sizes);
                sum = _e58 + (w * _e60);
                float _e63 = total;
                total = _e63 + w;
            }
        }
        float _e68 = total;
        if (_e68 <= 0.0) {
            float _e71 = bucket_level(slot_2, idx, density_1, locals, grid, _buffer_sizes);
            return _e71;
        }
        float _e72 = sum;
        float _e73 = total;
        return _e72 / _e73;
    }
    float _e75 = bucket_x(t_1, locals);
    float x = _e75 - 0.5;
    uint _e81 = locals.bins;
    uint b_2 = naga_f2u32(metal::clamp(metal::floor(x), 0.0, static_cast<float>(_e81) - 2.0));
    float f_1 = metal::clamp(x - static_cast<float>(b_2), 0.0, 1.0);
    float _e93 = bucket_level(slot_2, b_2, density_1, locals, grid, _buffer_sizes);
    float _e96 = bucket_level(slot_2, b_2 + 1u, density_1, locals, grid, _buffer_sizes);
    return metal::mix(_e93, _e96, f_1);
}

uint naga_mod(uint lhs, uint rhs) {
    return lhs % metal::select(rhs, 1u, rhs == 0u);
}

float field_level(
    VertexOut in_1,
    bool density_2,
    constant Locals& locals,
    device type_3 const& grid,
    constant _mslBufferSizes& _buffer_sizes
) {
    uint _e4 = locals.run_slabs;
    float n_2 = static_cast<float>(_e4);
    float jx = metal::clamp(metal::floor(in_1.slab - 0.5), 0.0, n_2 - 1.0);
    uint j0_ = naga_f2u32(jx);
    uint _e19 = locals.run_slabs;
    uint j1_ = metal::min(j0_ + 1u, _e19 - 1u);
    float fx = metal::clamp((in_1.slab - 0.5) - jx, 0.0, 1.0);
    uint _e32 = locals.first_slot;
    uint _e36 = locals.capacity;
    uint s0_ = naga_mod(_e32 + j0_, _e36);
    uint _e40 = locals.first_slot;
    uint _e44 = locals.capacity;
    uint s1_ = naga_mod(_e40 + j1_, _e44);
    float _e47 = read_level(s0_, in_1.t, density_2, locals, grid, _buffer_sizes);
    float _e49 = read_level(s1_, in_1.t, density_2, locals, grid, _buffer_sizes);
    return metal::mix(_e47, _e49, fx);
}

float heatmap_level(
    VertexOut in_2,
    constant Locals& locals,
    device type_3 const& grid,
    constant _mslBufferSizes& _buffer_sizes
) {
    float _e2 = field_level(in_2, false, locals, grid, _buffer_sizes);
    return _e2;
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
    float level_1,
    constant Cloud& cloud
) {
    uint _e3 = cloud.style;
    if (_e3 != 2u) {
        return level_1;
    }
    float _e11 = cloud.contours;
    float x_1 = metal::clamp(level_1, 0.0, 1.0) * _e11;
    float _e15 = cloud.contour_softness;
    float _e16 = metal::fwidth(x_1);
    float edge = metal::min(0.5, metal::max(_e15, _e16 * 0.5));
    float _e32 = cloud.contours;
    float terraces = (metal::floor(x_1) + metal::smoothstep(0.5 - edge, 0.5 + edge, metal::fract(x_1))) / _e32;
    float _e39 = metal::fwidth(x_1);
    float strength = (0.9 * metal::smoothstep(0.0, 1.0, x_1)) * (1.0 - metal::smoothstep(0.5, 1.5, _e39));
    return metal::mix(level_1, terraces, strength);
}

metal::float3 palette_color(
    float level_2,
    metal::texture2d<float, metal::access::sample> lut
) {
    uint levels = metal::uint2(lut.get_width(), lut.get_height()).x;
    float x_2 = (metal::clamp(level_2, 0.0, 1.0) * static_cast<float>(levels)) - 0.5;
    uint i_3 = naga_f2u32(metal::clamp(metal::floor(x_2), 0.0, static_cast<float>(levels - 1u)));
    uint clamped_lod_e22 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e22 = lut.read(metal::min(metal::uint2(metal::uint2(i_3, 0u)), metal::uint2(lut.get_width(clamped_lod_e22), lut.get_height(clamped_lod_e22)) - 1), clamped_lod_e22);
    metal::float3 a = _e22.xyz;
    if (x_2 < 0.0) {
        return (a * (x_2 + 0.5)) * 2.0;
    }
    uint clamped_lod_e40 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e40 = lut.read(metal::min(metal::uint2(metal::uint2(metal::min(i_3 + 1u, levels - 1u), 0u)), metal::uint2(lut.get_width(clamped_lod_e40), lut.get_height(clamped_lod_e40)) - 1), clamped_lod_e40);
    metal::float3 b_3 = _e40.xyz;
    return metal::mix(a, b_3, metal::fract(x_2));
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

metal::float3 cloud_hash3_(
    metal::int2 cell
) {
    uint n = {};
    n = (as_type<uint>(cell.x) * 2654435769u) ^ (as_type<uint>(cell.y) * 2246822507u);
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
    uint _e36 = n;
    uint _e44 = n;
    return metal::float3(static_cast<float>(_e30 & 1023u) / 1023.0, static_cast<float>((_e36 >> 10u) & 1023u) / 1023.0, static_cast<float>((_e44 >> 20u) & 1023u) / 1023.0);
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

Pile dome_octave(
    metal::float2 r,
    constant Cloud& cloud
) {
    float weight = 0.0;
    metal::float2 face = metal::float2(0.0);
    metal::float2 to_centre = metal::float2(0.0);
    metal::float2 rock = metal::float2(0.0);
    int j = -1;
    int i = {};
    Pile out = {};
    metal::float2 base_2 = metal::floor(r);
    uint2 loop_bound_1 = uint2(4294967295u);
    bool loop_init_1 = true;
    while(true) {
        if (metal::all(loop_bound_1 == uint2(0u))) { break; }
        loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
        if (!loop_init_1) {
            int _e121 = j;
            j = as_type<int>(as_type<uint>(_e121) + as_type<uint>(1));
        }
        loop_init_1 = false;
        int _e15 = j;
        if (_e15 <= 1) {
        } else {
            break;
        }
        {
            i = -1;
            uint2 loop_bound_2 = uint2(4294967295u);
            bool loop_init_2 = true;
            while(true) {
                if (metal::all(loop_bound_2 == uint2(0u))) { break; }
                loop_bound_2 -= uint2(loop_bound_2.y == 0u, 1u);
                if (!loop_init_2) {
                    int _e118 = i;
                    i = as_type<int>(as_type<uint>(_e118) + as_type<uint>(1));
                }
                loop_init_2 = false;
                int _e20 = i;
                if (_e20 <= 1) {
                } else {
                    break;
                }
                {
                    int _e24 = i;
                    int _e25 = j;
                    metal::int2 cell_3 = as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(base_2)) + as_type<metal::uint2>(metal::int2(_e24, _e25)));
                    metal::float3 _e28 = cloud_hash3_(cell_3);
                    int _e29 = i;
                    int _e31 = j;
                    metal::float2 centre = ((base_2 + metal::float2(static_cast<float>(_e29), static_cast<float>(_e31))) + metal::float2(0.5)) + ((_e28.xy - metal::float2(0.5)) * DOME_JITTER);
                    float _e52 = cloud.scale_variety;
                    float radius = metal::mix(DOME_RADIUS, metal::mix(DOME_RADIUS_MIN, DOME_RADIUS_MAX, _e28.z), _e52);
                    metal::float2 d = (r - centre) / metal::float2(radius);
                    float q = 1.0 - metal::dot(d, d);
                    if (q <= 0.0) {
                        continue;
                    }
                    float root = metal::sqrt(q);
                    float h = q * root;
                    float w_1 = metal::exp(DOME_UNION * h);
                    float _e67 = weight;
                    weight = _e67 + w_1;
                    metal::float2 _e69 = face;
                    face = _e69 + ((w_1 * -(((DOME_FACE * root) * radius))) * d);
                    metal::float2 _e77 = to_centre;
                    to_centre = _e77 + (w_1 * (centre - r));
                    float _e83 = cloud.scale_rock;
                    if (_e83 > 0.0) {
                        metal::float2 _e86 = rock;
                        float _e89 = cloud.time;
                        float _e103 = cloud.time;
                        rock = _e86 + (w_1 * metal::float2(metal::sin((_e89 * (0.2 + (0.3 * _e28.x))) + (_e28.y * 6.2831855)), metal::cos((_e103 * (0.25 + (0.2 * _e28.y))) + (_e28.x * 6.2831855))));
                    }
                }
            }
        }
    }
    float _e125 = weight;
    if (_e125 <= 0.0) {
        out.face = metal::float2(0.0);
        out.to_centre = metal::float2(0.0);
        out.rock = metal::float2(0.0);
        Pile _e137 = out;
        return _e137;
    }
    metal::float2 _e139 = face;
    float _e140 = weight;
    out.face = _e139 / metal::float2(_e140);
    metal::float2 _e144 = to_centre;
    float _e145 = weight;
    out.to_centre = _e144 / metal::float2(_e145);
    metal::float2 _e149 = rock;
    float _e150 = weight;
    out.rock = _e149 / metal::float2(_e150);
    Pile _e153 = out;
    return _e153;
}

Pile cloud_domes(
    metal::float2 r_1,
    constant Cloud& cloud
) {
    Pile out_1 = {};
    Pile _e1 = dome_octave(r_1, cloud);
    Pile _e8 = dome_octave((r_1 * DOME_LACUNARITY) + metal::float2(17.3, 5.9), cloud);
    out_1.face = (_e1.face + (0.46199998 * _e8.face)) / metal::float2(1.22);
    out_1.to_centre = _e1.to_centre;
    out_1.rock = (_e1.rock + (DOME_FINE_GAIN * _e8.rock)) / metal::float2(1.22);
    Pile _e29 = out_1;
    return _e29;
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
    metal::float2 tilt = {};
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
    metal::float2 pt_3 = (position_1 / metal::float2(_e21)) - _e26;
    float _e30 = cloud.cloud_scale;
    float units = 5.0 / _e30;
    metal::float2 _e35 = cloud.size;
    float _e42 = cloud.size.y;
    metal::float2 _e48 = cloud.drift;
    metal::float2 q_1 = (((pt_3 - (_e35 * 0.5)) / metal::float2(_e42)) * units) + _e48;
    float _e52 = cloud.scale_size;
    float scale_units = 6.0 / _e52;
    float _e58 = cloud.size.y;
    float scale_points = (_e58 / units) / scale_units;
    Pile _e62 = cloud_domes(q_1 * scale_units, cloud);
    metal::float2 face_1 = _e62.face;
    float _e66 = cloud.scale_refract;
    float bend = _e66 * scale_points;
    float _e74 = cloud.scale_facet;
    metal::float2 lookup = metal::mix(-(face_1) * bend, _e62.to_centre * scale_points, _e74);
    float _e77 = cloud_light(pt_3 + lookup, close_light, wide_light, cloud_sampler, cloud);
    metal::float2 reach = metal::float2(scale_points * 0.75, 0.0);
    float _e84 = cloud_light(pt_3 + reach.xy, close_light, wide_light, cloud_sampler, cloud);
    float _e87 = cloud_light(pt_3 - reach.xy, close_light, wide_light, cloud_sampler, cloud);
    float _e91 = cloud_light(pt_3 + reach.yx, close_light, wide_light, cloud_sampler, cloud);
    float _e94 = cloud_light(pt_3 - reach.yx, close_light, wide_light, cloud_sampler, cloud);
    metal::float2 grad = metal::float2(_e84 - _e87, _e91 - _e94);
    metal::float2 lean = grad * (SUN_LEAN / (SUN_KNEE + metal::length(grad)));
    float relief = cloud.scale_relief;
    tilt = face_1;
    float _e109 = cloud.scale_rock;
    if (_e109 > 0.0) {
        metal::float2 _e112 = tilt;
        float _e116 = cloud.scale_rock;
        tilt = _e112 + (_e62.rock * (_e116 * ROCK_TILT));
    }
    metal::float2 _e121 = tilt;
    metal::float3 normal = metal::normalize(metal::float3(-(_e121) * relief, 1.0));
    metal::float3 sun = metal::normalize(metal::float3(lean, 1.0));
    float lambert = metal::max(metal::dot(normal, sun), 0.0) / sun.z;
    float _e137 = cloud.scale_shade_floor;
    float diffuse = metal::mix(_e137, 1.0, lambert);
    float lit = (_e77 * diffuse) * CLOUD_SHADE;
    metal::float3 _e146 = palette_color(metal::clamp(lit, 0.0, 1.0), lut);
    float _e149 = cloud.cloud_depth;
    return metal::mix(base, _e146, _e149);
}

metal::float3 wash_hash(
    metal::int2 cell_1,
    uint salt
) {
    uint n_1 = {};
    n_1 = (as_type<uint>(cell_1.x) * 2654435769u) ^ (as_type<uint>(cell_1.y) * 2246822507u);
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
    uint salt_1
) {
    metal::float2 b_4 = metal::floor(p);
    metal::float2 f_2 = p - b_4;
    metal::float2 t_2 = (f_2 * f_2) * (metal::float2(3.0) - (2.0 * f_2));
    metal::int2 i_4 = naga_f2i32(b_4);
    metal::float3 _e12 = wash_hash(i_4, salt_1);
    float n00_ = _e12.x;
    metal::float3 _e18 = wash_hash(as_type<metal::int2>(as_type<metal::uint2>(i_4) + as_type<metal::uint2>(metal::int2(1, 0))), salt_1);
    float n10_ = _e18.x;
    metal::float3 _e24 = wash_hash(as_type<metal::int2>(as_type<metal::uint2>(i_4) + as_type<metal::uint2>(metal::int2(0, 1))), salt_1);
    float n01_ = _e24.x;
    metal::float3 _e30 = wash_hash(as_type<metal::int2>(as_type<metal::uint2>(i_4) + as_type<metal::uint2>(metal::int2(1, 1))), salt_1);
    float n11_ = _e30.x;
    return metal::mix(metal::mix(n00_, n10_, t_2.x), metal::mix(n01_, n11_, t_2.x), t_2.y);
}

float wash_fbm(
    metal::float2 p_1,
    uint salt_2
) {
    float _e2 = wash_noise(p_1, salt_2);
    float _e11 = wash_noise((p_1 * 2.07) + metal::float2(13.1, -7.3), salt_2 + 31u);
    return (_e2 + (0.5 * _e11)) / 1.5;
}

Glob wash_glob(
    metal::int2 cell_2,
    uint salt_3,
    metal::float2 r_2,
    float wob,
    float occupancy,
    constant Cloud& cloud
) {
    metal::float2 offset = {};
    Glob out_2 = {};
    metal::float3 _e5 = wash_hash(cell_2, salt_3);
    metal::float3 _e8 = wash_hash(cell_2, salt_3 + 77u);
    offset = (_e5.xy - metal::float2(0.5)) * WASH_JITTER;
    float _e18 = cloud.wash_wander;
    if (_e18 > 0.0) {
        float _e23 = cloud.time;
        float _e32 = cloud.wash_wander;
        float turn = (_e23 * (0.05 + (0.12 * _e8.z))) * _e32;
        float c = metal::cos(turn);
        float s = metal::sin(turn);
        float _e37 = offset.x;
        float _e40 = offset.y;
        float _e44 = offset.x;
        float _e47 = offset.y;
        offset = metal::float2((_e37 * c) - (_e40 * s), (_e44 * s) + (_e47 * c));
    }
    metal::float2 _e55 = offset;
    metal::float2 centre_1 = (static_cast<metal::float2>(cell_2) + metal::float2(0.5)) + _e55;
    float _e64 = cloud.wash_variety;
    float radius_1 = metal::mix(WASH_RADIUS, metal::mix(WASH_RADIUS_MIN, WASH_RADIUS_MAX, _e5.z), _e64);
    out_2.centre = centre_1;
    out_2.order = _e8.x;
    bool present = _e8.y < occupancy;
    out_2.edge = present ? ((metal::length(r_2 - centre_1) / radius_1) + wob) : 1000000000.0;
    Glob _e79 = out_2;
    return _e79;
}

Wash wash_scan(
    metal::float2 r_3,
    uint salt_4,
    float occupancy_1,
    float wob_1,
    constant Cloud& cloud
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
    out_3.tau = 0.0;
    front_a = r_3;
    front_b = r_3;
    metal::int2 base_3 = naga_f2i32(metal::floor(r_3));
    uint2 loop_bound_3 = uint2(4294967295u);
    bool loop_init_3 = true;
    while(true) {
        if (metal::all(loop_bound_3 == uint2(0u))) { break; }
        loop_bound_3 -= uint2(loop_bound_3.y == 0u, 1u);
        if (!loop_init_3) {
            int _e108 = j_1;
            j_1 = as_type<int>(as_type<uint>(_e108) + as_type<uint>(1));
        }
        loop_init_3 = false;
        int _e34 = j_1;
        if (_e34 <= WASH_RING) {
        } else {
            break;
        }
        {
            i_1 = -2;
            uint2 loop_bound_4 = uint2(4294967295u);
            bool loop_init_4 = true;
            while(true) {
                if (metal::all(loop_bound_4 == uint2(0u))) { break; }
                loop_bound_4 -= uint2(loop_bound_4.y == 0u, 1u);
                if (!loop_init_4) {
                    int _e105 = i_1;
                    i_1 = as_type<int>(as_type<uint>(_e105) + as_type<uint>(1));
                }
                loop_init_4 = false;
                int _e39 = i_1;
                if (_e39 <= WASH_RING) {
                } else {
                    break;
                }
                {
                    int _e42 = i_1;
                    int _e43 = j_1;
                    Glob _e46 = wash_glob(as_type<metal::int2>(as_type<metal::uint2>(base_3) + as_type<metal::uint2>(metal::int2(_e42, _e43))), salt_4, r_3, wob_1, occupancy_1, cloud);
                    float prox = 1.0 - _e46.edge;
                    float _e52 = out_3.cover;
                    out_3.cover = metal::max(_e52, metal::clamp(prox / 0.05, 0.0, 1.0));
                    float body = metal::clamp(prox / 0.1, 0.0, 1.0);
                    float _e65 = out_3.tau;
                    out_3.tau = _e65 + ((body * body) * (3.0 - (2.0 * body)));
                    if (_e46.edge < 1.0) {
                        float _e77 = best;
                        if (_e46.order > _e77) {
                            float _e79 = best;
                            second = _e79;
                            metal::float2 _e82 = out_3.centre;
                            out_3.under = _e82;
                            best = _e46.order;
                            out_3.centre = _e46.centre;
                            out_3.edge = _e46.edge;
                        } else {
                            float _e89 = second;
                            if (_e46.order > _e89) {
                                second = _e46.order;
                                out_3.under = _e46.centre;
                            }
                        }
                    } else {
                        float _e94 = near_a;
                        if (prox > _e94) {
                            float _e96 = near_a;
                            near_b = _e96;
                            float _e97 = order_a;
                            order_b = _e97;
                            metal::float2 _e98 = front_a;
                            front_b = _e98;
                            near_a = prox;
                            order_a = _e46.order;
                            front_a = _e46.centre;
                        } else {
                            float _e101 = near_b;
                            if (prox > _e101) {
                                near_b = prox;
                                order_b = _e46.order;
                                front_b = _e46.centre;
                            }
                        }
                    }
                }
            }
        }
    }
    float _e111 = order_b;
    float _e112 = best;
    if (_e111 > _e112) {
        float _e115 = near_b;
        out_3.near = _e115;
        metal::float2 _e117 = front_b;
        out_3.front = _e117;
    }
    float _e118 = order_a;
    float _e119 = best;
    if (_e118 > _e119) {
        float _e122 = near_a;
        out_3.near = _e122;
        metal::float2 _e124 = front_a;
        out_3.front = _e124;
    }
    Wash _e125 = out_3;
    return _e125;
}

float wash_light(
    metal::float2 pt_1,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    metal::float2 _e3 = cloud.size;
    metal::float2 uv_2 = pt_1 / _e3;
    metal::float4 _e8 = close_light.sample(cloud_sampler, uv_2, metal::level(0.0));
    float material_2 = _e8.x;
    float _e12 = cloud.wash_soften;
    if (_e12 <= 0.0) {
        return material_2;
    }
    metal::float4 _e18 = wide_light.sample(cloud_sampler, uv_2, metal::level(0.0));
    float _e20 = density_decode(_e18.x);
    float _e23 = cloud.wash_soften;
    return metal::mix(material_2, _e20, _e23);
}

float wash_tone(
    Wash f,
    metal::float2 r_4,
    float pane_per_cell,
    metal::float2 pt_2,
    float average_pile,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    metal::float2 look = {};
    float fa = {};
    float bl = {};
    float pigment = {};
    float fuzz = cloud.wash_fuzz;
    float feather = 0.1 + (0.8 * fuzz);
    float bleed = 0.12 + (0.78 * fuzz);
    float _e19 = cloud.wash_pool;
    float tide = (WASH_POOL * _e19) * (1.0 - (0.75 * fuzz));
    float surf = WASH_SURF * (1.0 - (0.45 * fuzz));
    look = f.centre;
    fa = metal::clamp((f.edge - (1.0 - feather)) / feather, 0.0, 1.0);
    float _e43 = fa;
    float _e44 = fa;
    float _e46 = fa;
    fa = ((_e43 * _e44) * (3.0 - (2.0 * _e46))) * 0.5;
    metal::float2 _e54 = look;
    float _e56 = fa;
    look = metal::mix(_e54, f.under, _e56);
    bl = metal::clamp((f.near + bleed) / bleed, 0.0, 1.0);
    float _e65 = bl;
    float _e66 = bl;
    float _e68 = bl;
    bl = ((_e65 * _e66) * (3.0 - (2.0 * _e68))) * 0.5;
    metal::float2 _e76 = look;
    float _e78 = bl;
    look = metal::mix(_e76, f.front, _e78);
    metal::float2 _e80 = look;
    float _e85 = cloud.wash_refract;
    float _e88 = wash_light(pt_2 + (((_e80 - r_4) * pane_per_cell) * _e85), close_light, wide_light, cloud_sampler, cloud);
    float paper = metal::clamp((WASH_PIVOT + (WASH_LIFT_A * (_e88 - WASH_PIVOT))) + WASH_LIFT_B, 0.0, 1.0);
    float rim = metal::clamp(f.edge, 0.0, 1.0);
    pigment = (surf * rim) * rim;
    float crescent = metal::clamp((f.near + WASH_POOL_WIDTH) / WASH_POOL_WIDTH, 0.0, 1.0);
    float _e115 = pigment;
    pigment = _e115 + ((tide * crescent) * crescent);
    float _e119 = pigment;
    float _e123 = cloud.wash_grain;
    pigment = _e119 + ((WASH_GRAIN * _e123) * metal::max(f.tau - average_pile, 0.0));
    float _e131 = pigment;
    float pig = metal::max(_e131, 0.0);
    return paper - (pig * (WASH_PIG_DEPTH + (0.65 * paper)));
}

float wash_average_pile(
    float occupancy_2,
    constant Cloud& cloud
) {
    float _e5 = cloud.wash_variety;
    float lo_1 = metal::mix(WASH_RADIUS, WASH_RADIUS_MIN, _e5);
    float _e11 = cloud.wash_variety;
    float hi_1 = metal::mix(WASH_RADIUS, WASH_RADIUS_MAX, _e11);
    return ((occupancy_2 * 3.1415927) * (((lo_1 * lo_1) + (lo_1 * hi_1)) + (hi_1 * hi_1))) / 3.0;
}

metal::float3 wash_clouds(
    metal::float3 base_1,
    metal::float2 position_2,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    bool local_1 = {};
    metal::float2 warped = {};
    float wob_2 = 0.0;
    float tone = {};
    float _e4 = cloud.cloud_depth;
    if (!((_e4 <= 0.0))) {
        metal::float2 _e12 = cloud.step;
        local_1 = metal::all(_e12 == metal::float2(0.0));
    } else {
        local_1 = true;
    }
    bool _e18 = local_1;
    if (_e18) {
        return base_1;
    }
    float _e21 = cloud.ppp;
    metal::float2 _e26 = cloud.origin;
    metal::float2 pt_4 = (position_2 / metal::float2(_e21)) - _e26;
    float _e30 = cloud.cloud_scale;
    float units_1 = 5.0 / _e30;
    metal::float2 _e35 = cloud.size;
    float _e42 = cloud.size.y;
    metal::float2 _e48 = cloud.drift;
    metal::float2 q_2 = (((pt_4 - (_e35 * 0.5)) / metal::float2(_e42)) * units_1) + _e48;
    float _e53 = cloud.wash_size;
    float cells = WASH_CELLS / _e53;
    float _e58 = cloud.size.y;
    float pane_per_cell_1 = (_e58 / units_1) / cells;
    metal::float2 r_5 = q_2 * cells;
    warped = r_5;
    float _e65 = cloud.wash_lobe;
    if (_e65 > 0.0) {
        float _e71 = cloud.wash_lobe;
        float amp = WASH_WARP * _e71;
        metal::float2 _e73 = warped;
        float _e79 = wash_fbm(r_5 * WASH_WARP_SCALE, 71u);
        float _e89 = wash_fbm((r_5 * WASH_WARP_SCALE) + metal::float2(37.0, -19.0), 73u);
        warped = _e73 + ((amp * 2.0) * metal::float2(_e79 - 0.5, _e89 - 0.5));
    }
    float _e99 = cloud.wash_ragged;
    if (_e99 > 0.0) {
        float _e105 = cloud.wash_ragged;
        float _e110 = wash_fbm(r_5 * WASH_RAGGED_SCALE, 41u);
        wob_2 = (WASH_RAGGED * _e105) * (_e110 - 1.0);
    }
    metal::float2 _e114 = warped;
    float _e117 = wob_2;
    Wash _e118 = wash_scan(_e114, 1u, 1.0, _e117, cloud);
    metal::float2 _e119 = warped;
    float _e121 = wash_average_pile(1.0, cloud);
    float _e122 = wash_tone(_e118, _e119, pane_per_cell_1, pt_4, _e121, close_light, wide_light, cloud_sampler, cloud);
    tone = _e122;
    float _e126 = cloud.wash_layers;
    if (_e126 > 0.0) {
        metal::float2 _e129 = warped;
        metal::float2 fine_r = (_e129 * WASH_LACUNARITY) + metal::float2(17.3, 5.9);
        float _e138 = wob_2;
        Wash _e139 = wash_scan(fine_r, 2u, WASH_FINE_OCCUPANCY, _e138, cloud);
        float _e143 = wash_average_pile(WASH_FINE_OCCUPANCY, cloud);
        float _e144 = wash_tone(_e139, fine_r, pane_per_cell_1 / WASH_LACUNARITY, pt_4, _e143, close_light, wide_light, cloud_sampler, cloud);
        float _e145 = tone;
        float _e148 = cloud.wash_layers;
        tone = metal::mix(_e145, _e144, _e148 * _e139.cover);
    }
    float _e152 = tone;
    metal::float3 _e156 = palette_color(metal::clamp(_e152, WASH_TONE_FLOOR, 1.0), lut);
    float _e159 = cloud.cloud_depth;
    return metal::mix(base_1, _e156, _e159);
}

metal::float4 clouded(
    float level_3,
    metal::float2 position_3,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    metal::float4 _e2 = density_color(level_3, lut, cloud);
    uint _e5 = cloud.cloud_style;
    if (_e5 == 1u) {
        metal::float3 _e9 = wash_clouds(_e2.xyz, position_3, lut, close_light, wide_light, cloud_sampler, cloud);
        return metal::float4(_e9, 1.0);
    }
    metal::float3 _e13 = scale_clouds(_e2.xyz, position_3, lut, close_light, wide_light, cloud_sampler, cloud);
    return metal::float4(_e13, 1.0);
}

metal::float4 cloud_color(
    VertexOut in_3,
    constant Locals& locals,
    device type_3 const& grid,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    constant _mslBufferSizes& _buffer_sizes
) {
    float _e1 = heatmap_level(in_3, locals, grid, _buffer_sizes);
    float _e4 = baked_density(in_3.position.xy, close_light, cloud_sampler, cloud);
    float _e5 = smoothed_level(_e1, _e4, cloud);
    metal::float4 _e8 = clouded(_e5, in_3.position.xy, lut, close_light, wide_light, cloud_sampler, cloud);
    return _e8;
}

struct fs_cloud_gammaInput {
    float slab [[user(loc0), center_perspective]];
    float t [[user(loc1), center_perspective]];
};
struct fs_cloud_gammaOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_cloud_gammaOutput fs_cloud_gamma(
  fs_cloud_gammaInput varyings [[stage_in]]
, metal::float4 position_4 [[position]]
, constant Locals& locals [[buffer(0)]]
, device type_3 const& grid [[buffer(1)]]
, metal::texture2d<float, metal::access::sample> lut [[texture(0)]]
, metal::texture2d<float, metal::access::sample> close_light [[texture(1)]]
, metal::texture2d<float, metal::access::sample> wide_light [[texture(2)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(2)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    const VertexOut in = { position_4, varyings.slab, varyings.t };
    metal::float4 _e1 = cloud_color(in, locals, grid, lut, close_light, wide_light, cloud_sampler, cloud, _buffer_sizes);
    return fs_cloud_gammaOutput { _e1 };
}
