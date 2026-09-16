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
    float cloud_cover;
    float scale_size;
    float scale_overlap;
    float scale_glint;
    float cloud_ambient;
    float _pad2_;
    float _pad3_;
};
struct Puffs {
    float depth;
    char _pad1[4];
    metal::float2 slope;
    float body;
    char _pad3[4];
    metal::float2 body_slope;
    metal::float2 lit_centre;
    float lit_weight;
    char _pad6[4];
};
constant float PUFF_JITTER = 0.6;
constant int PUFF_OCTAVES = 3;

uint stored(
    uint slot,
    uint bucket,
    constant Locals& locals,
    device type_3 const& grid,
    constant _mslBufferSizes& _buffer_sizes
) {
    uint _e4 = locals.stride;
    uint i_1 = (slot * _e4) + bucket;
    uint _e11 = grid[metal::min(unsigned(i_1 >> 2u), (_buffer_sizes.size1 - 0 - 4) / 4)];
    return (_e11 >> ((i_1 & 3u) * 8u)) & 255u;
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
    float f = metal::clamp(x - static_cast<float>(b_2), 0.0, 1.0);
    float _e93 = bucket_level(slot_2, b_2, density_1, locals, grid, _buffer_sizes);
    float _e96 = bucket_level(slot_2, b_2 + 1u, density_1, locals, grid, _buffer_sizes);
    return metal::mix(_e93, _e96, f);
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
    float n_1 = static_cast<float>(_e4);
    float jx = metal::clamp(metal::floor(in_1.slab - 0.5), 0.0, n_1 - 1.0);
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
    uint i_2 = naga_f2u32(metal::clamp(metal::floor(x_2), 0.0, static_cast<float>(levels - 1u)));
    uint clamped_lod_e22 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e22 = lut.read(metal::min(metal::uint2(metal::uint2(i_2, 0u)), metal::uint2(lut.get_width(clamped_lod_e22), lut.get_height(clamped_lod_e22)) - 1), clamped_lod_e22);
    metal::float3 a = _e22.xyz;
    if (x_2 < 0.0) {
        return (a * (x_2 + 0.5)) * 2.0;
    }
    uint clamped_lod_e40 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e40 = lut.read(metal::min(metal::uint2(metal::uint2(metal::min(i_2 + 1u, levels - 1u), 0u)), metal::uint2(lut.get_width(clamped_lod_e40), lut.get_height(clamped_lod_e40)) - 1), clamped_lod_e40);
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
    float t_2 = metal::fract(((_e9 * turns) * 0.001) + phase);
    float ramp = metal::abs((t_2 * 2.0) - 1.0);
    return (((ramp * ramp) * (3.0 - (2.0 * ramp))) * 2.0) - 1.0;
}

float wrapped_light(
    float cosine
) {
    return metal::pow(metal::max((cosine * 0.5) + 0.5, 0.0), 1.5);
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

Puffs puff_field(
    metal::float2 p,
    float lacunarity,
    float radius,
    constant Cloud& cloud
) {
    Puffs out = {};
    float freq = 1.0;
    float amp = 1.0;
    metal::float2 lit_acc = metal::float2(0.0);
    int octave = 0;
    int j = {};
    int i = {};
    out = Puffs {0.0, {}, metal::float2(0.0), 0.0, {}, metal::float2(0.0), p, 0.0};
    float inv_r2_ = 1.0 / (radius * radius);
    uint2 loop_bound_1 = uint2(4294967295u);
    bool loop_init_1 = true;
    while(true) {
        if (metal::all(loop_bound_1 == uint2(0u))) { break; }
        loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
        if (!loop_init_1) {
            int _e154 = octave;
            octave = as_type<int>(as_type<uint>(_e154) + as_type<uint>(1));
        }
        loop_init_1 = false;
        int _e24 = octave;
        if (_e24 < PUFF_OCTAVES) {
        } else {
            break;
        }
        {
            int _e27 = octave;
            int _e31 = octave;
            metal::float2 shift_1 = metal::float2(static_cast<float>(_e27) * 31.7, static_cast<float>(_e31) * -17.3);
            float _e36 = freq;
            metal::float2 r = (p * _e36) + shift_1;
            metal::int2 home = naga_f2i32(metal::floor(r));
            int _e41 = octave;
            uint salt_1 = static_cast<uint>(_e41) * 2654435769u;
            j = -1;
            uint2 loop_bound_2 = uint2(4294967295u);
            bool loop_init_2 = true;
            while(true) {
                if (metal::all(loop_bound_2 == uint2(0u))) { break; }
                loop_bound_2 -= uint2(loop_bound_2.y == 0u, 1u);
                if (!loop_init_2) {
                    int _e146 = j;
                    j = as_type<int>(as_type<uint>(_e146) + as_type<uint>(1));
                }
                loop_init_2 = false;
                int _e47 = j;
                if (_e47 <= 1) {
                } else {
                    break;
                }
                {
                    i = -1;
                    uint2 loop_bound_3 = uint2(4294967295u);
                    bool loop_init_3 = true;
                    while(true) {
                        if (metal::all(loop_bound_3 == uint2(0u))) { break; }
                        loop_bound_3 -= uint2(loop_bound_3.y == 0u, 1u);
                        if (!loop_init_3) {
                            int _e143 = i;
                            i = as_type<int>(as_type<uint>(_e143) + as_type<uint>(1));
                        }
                        loop_init_3 = false;
                        int _e52 = i;
                        if (_e52 <= 1) {
                        } else {
                            break;
                        }
                        {
                            int _e55 = i;
                            int _e56 = j;
                            metal::int2 c = as_type<metal::int2>(as_type<metal::uint2>(home) + as_type<metal::uint2>(metal::int2(_e55, _e56)));
                            uint _e59 = cloud_bits(c, salt_1);
                            uint _e62 = cloud_bits(c, salt_1 ^ 3266489909u);
                            float _e64 = cloud_slice(_e59, 0u);
                            float _e66 = cloud_slice(_e59, 10u);
                            metal::float2 place = metal::float2(_e64, _e66) - metal::float2(0.5);
                            float _e72 = cloud_slice(_e62, 0u);
                            float _e74 = cloud_slice(_e62, 20u);
                            float _e75 = puff_wave(_e72, _e74, cloud);
                            float _e77 = cloud_slice(_e62, 10u);
                            float _e79 = cloud_slice(_e59, 20u);
                            float _e80 = puff_wave(_e77, _e79, cloud);
                            metal::float2 wander = metal::float2(_e75, _e80);
                            metal::float2 centre = (static_cast<metal::float2>(c) + metal::float2(0.5)) + ((place + (wander * 0.18)) * PUFF_JITTER);
                            metal::float2 x_3 = r - centre;
                            float u = 1.0 - (metal::dot(x_3, x_3) * inv_r2_);
                            if (u <= 0.0) {
                                continue;
                            }
                            float _e100 = cloud_slice(_e59, 20u);
                            float _e102 = amp;
                            float weight = (_e100 * _e100) * _e102;
                            float lump = (u * u) * weight;
                            float _e109 = freq;
                            metal::float2 lump_slope = ((((-4.0 * u) * weight) * _e109) * inv_r2_) * x_3;
                            float _e114 = out.depth;
                            out.depth = _e114 + lump;
                            metal::float2 _e117 = out.slope;
                            out.slope = _e117 + lump_slope;
                            int _e119 = octave;
                            if (_e119 == 0) {
                                float _e123 = out.body;
                                out.body = _e123 + lump;
                                metal::float2 _e126 = out.body_slope;
                                out.body_slope = _e126 + lump_slope;
                            }
                            int _e128 = octave;
                            if (_e128 == 2) {
                                float u2_ = u * u;
                                float reach = u2_ * u2_;
                                metal::float2 _e133 = lit_acc;
                                float _e136 = freq;
                                lit_acc = _e133 + ((reach * (centre - shift_1)) / metal::float2(_e136));
                                float _e141 = out.lit_weight;
                                out.lit_weight = _e141 + reach;
                            }
                        }
                    }
                }
            }
            float _e149 = freq;
            freq = _e149 * lacunarity;
            float _e151 = amp;
            amp = _e151 * 0.5;
        }
    }
    float _e158 = out.lit_weight;
    if (_e158 > 0.0) {
        metal::float2 _e162 = lit_acc;
        float _e164 = out.lit_weight;
        out.lit_centre = _e162 / metal::float2(_e164);
    }
    Puffs _e167 = out;
    return _e167;
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
    metal::float2 pt_1 = (position_1 / metal::float2(_e21)) - _e26;
    float _e30 = cloud.cloud_scale;
    float units = 5.0 / _e30;
    metal::float2 _e35 = cloud.size;
    float _e42 = cloud.size.y;
    metal::float2 _e48 = cloud.drift;
    metal::float2 q = (((pt_1 - (_e35 * 0.5)) / metal::float2(_e42)) * units) + _e48;
    float _e52 = cloud.scale_size;
    float lacunarity_1 = metal::max(1.2, metal::sqrt(4.0 / _e52));
    float _e60 = cloud.scale_overlap;
    Puffs _e61 = puff_field(q, lacunarity_1, _e60, cloud);
    float _e64 = cloud.cloud_cover;
    float sill = metal::mix(0.9, -0.35, _e64);
    float alpha = metal::smoothstep(sill, sill + 0.8, _e61.depth);
    if (alpha <= 0.002) {
        return base;
    }
    float _e77 = cloud.size.y;
    float cloud_points = _e77 / units;
    metal::float2 reach_1 = metal::float2(cloud_points * 0.5, 0.0);
    float _e85 = cloud_light(pt_1 + reach_1.xy, close_light, wide_light, cloud_sampler, cloud);
    float _e88 = cloud_light(pt_1 - reach_1.xy, close_light, wide_light, cloud_sampler, cloud);
    float _e92 = cloud_light(pt_1 + reach_1.yx, close_light, wide_light, cloud_sampler, cloud);
    float _e95 = cloud_light(pt_1 - reach_1.yx, close_light, wide_light, cloud_sampler, cloud);
    metal::float2 grad = metal::float2(_e85 - _e88, _e92 - _e95);
    float magnitude = metal::length(grad);
    float directed = metal::smoothstep(0.0, 0.03, magnitude);
    metal::float2 toward = metal::normalize(metal::mix(metal::float2(0.55, -0.83), grad / metal::float2(metal::max(magnitude, 0.00001)), directed));
    float aimed = 0.5 + (0.5 * directed);
    metal::float2 _e118 = cloud.drift;
    float _e125 = cloud.size.y;
    metal::float2 _e129 = cloud.size;
    metal::float2 centre_pt = (((_e61.lit_centre - _e118) / metal::float2(units)) * _e125) + (_e129 * 0.5);
    float quantized = 0.6 * metal::smoothstep(0.0, 0.1, _e61.lit_weight);
    float _e139 = cloud_light(pt_1, close_light, wide_light, cloud_sampler, cloud);
    float _e140 = cloud_light(centre_pt, close_light, wide_light, cloud_sampler, cloud);
    float light = metal::mix(_e139, _e140, quantized);
    float _e146 = cloud.scale_glint;
    metal::float3 normal = metal::normalize(metal::float3(-(_e61.slope) * (1.6 * _e146), 1.0));
    metal::float3 sun = metal::normalize(metal::float3(toward * 0.7, 0.7));
    float _e159 = wrapped_light(metal::dot(normal, sun));
    float _e161 = wrapped_light(sun.z);
    float diffuse = _e159 / _e161;
    metal::float3 half_1 = metal::normalize(sun + metal::float3(0.0, 0.0, 1.0));
    float flat_glint = metal::pow(half_1.z, 24.0);
    float glint = (metal::max(metal::pow(metal::max(metal::dot(normal, half_1), 0.0), 24.0) - flat_glint, 0.0) / (1.0 - flat_glint)) * aimed;
    float rim = metal::clamp(metal::dot(-(_e61.body_slope), toward) * 0.8, 0.0, 1.0) * aimed;
    float thickness = metal::clamp(_e61.depth * 1.1, 0.0, 1.4);
    float shading = (diffuse * (0.35 + (0.75 * thickness))) * (0.8 + (0.5 * rim));
    float _e213 = cloud.cloud_ambient;
    float level_5 = metal::clamp((light * 0.9) + _e213, 0.0, 1.0);
    metal::float3 _e218 = palette_color(level_5, lut);
    float _e222 = cloud.scale_glint;
    metal::float3 body = (_e218 * shading) + metal::float3(((glint * _e222) * level_5) * 0.5);
    float _e231 = cloud.cloud_depth;
    return metal::mix(base, body, _e231 * alpha);
}

metal::float4 clouded(
    float level_3,
    metal::float2 position_2,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    metal::float4 _e2 = density_color(level_3, lut, cloud);
    metal::float3 _e4 = scale_clouds(_e2.xyz, position_2, lut, close_light, wide_light, cloud_sampler, cloud);
    return metal::float4(_e4, 1.0);
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

struct fs_cloud_linearInput {
    float slab [[user(loc0), center_perspective]];
    float t [[user(loc1), center_perspective]];
};
struct fs_cloud_linearOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_cloud_linearOutput fs_cloud_linear(
  fs_cloud_linearInput varyings [[stage_in]]
, metal::float4 position_3 [[position]]
, constant Locals& locals [[buffer(0)]]
, device type_3 const& grid [[buffer(1)]]
, metal::texture2d<float, metal::access::sample> lut [[texture(0)]]
, metal::texture2d<float, metal::access::sample> close_light [[texture(1)]]
, metal::texture2d<float, metal::access::sample> wide_light [[texture(2)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(2)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    const VertexOut in = { position_3, varyings.slab, varyings.t };
    metal::float4 _e1 = cloud_color(in, locals, grid, lut, close_light, wide_light, cloud_sampler, cloud, _buffer_sizes);
    metal::float3 _e3 = linear_from_gamma_rgb(_e1.xyz);
    return fs_cloud_linearOutput { metal::float4(_e3, _e1.w) };
}
