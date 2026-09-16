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
    float broad_depth;
    float body;
    metal::float2 body_slope;
    metal::float2 lit_centre;
    float lit_weight;
    char _pad7[4];
};
struct Backlight {
    float glow;
    char _pad1[4];
    metal::float2 toward;
    float aimed;
    char _pad3[4];
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
    float broad = 1.0;
    float shape = 1.0;
    metal::float2 lit_acc = metal::float2(0.0);
    int octave = 0;
    int j = {};
    int i = {};
    metal::float2 wander = {};
    out = Puffs {0.0, {}, metal::float2(0.0), 0.0, 0.0, metal::float2(0.0), p, 0.0};
    uint2 loop_bound_1 = uint2(4294967295u);
    bool loop_init_1 = true;
    while(true) {
        if (metal::all(loop_bound_1 == uint2(0u))) { break; }
        loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
        if (!loop_init_1) {
            int _e191 = octave;
            octave = as_type<int>(as_type<uint>(_e191) + as_type<uint>(1));
        }
        loop_init_1 = false;
        int _e26 = octave;
        if (_e26 < PUFF_OCTAVES) {
        } else {
            break;
        }
        {
            int _e29 = octave;
            int _e33 = octave;
            metal::float2 shift_1 = metal::float2(static_cast<float>(_e29) * 31.7, static_cast<float>(_e33) * -17.3);
            float _e38 = freq;
            metal::float2 r = (p * _e38) + shift_1;
            metal::int2 home = naga_f2i32(metal::floor(r));
            int _e43 = octave;
            uint salt_1 = static_cast<uint>(_e43) * 2654435769u;
            int _e47 = octave;
            float reach_r = metal::min(radius * (1.0 + (0.18 * static_cast<float>(_e47))), 1.13);
            float inv_r2_ = 1.0 / (reach_r * reach_r);
            j = -1;
            uint2 loop_bound_2 = uint2(4294967295u);
            bool loop_init_2 = true;
            while(true) {
                if (metal::all(loop_bound_2 == uint2(0u))) { break; }
                loop_bound_2 -= uint2(loop_bound_2.y == 0u, 1u);
                if (!loop_init_2) {
                    int _e178 = j;
                    j = as_type<int>(as_type<uint>(_e178) + as_type<uint>(1));
                }
                loop_init_2 = false;
                int _e61 = j;
                if (_e61 <= 1) {
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
                            int _e175 = i;
                            i = as_type<int>(as_type<uint>(_e175) + as_type<uint>(1));
                        }
                        loop_init_3 = false;
                        int _e66 = i;
                        if (_e66 <= 1) {
                        } else {
                            break;
                        }
                        {
                            int _e69 = i;
                            int _e70 = j;
                            metal::int2 c = as_type<metal::int2>(as_type<metal::uint2>(home) + as_type<metal::uint2>(metal::int2(_e69, _e70)));
                            uint _e73 = cloud_bits(c, salt_1);
                            float _e75 = cloud_slice(_e73, 0u);
                            float _e77 = cloud_slice(_e73, 10u);
                            metal::float2 place = metal::float2(_e75, _e77) - metal::float2(0.5);
                            wander = metal::float2(0.0);
                            int _e85 = octave;
                            if (_e85 == 0) {
                                uint _e90 = cloud_bits(c, salt_1 ^ 3266489909u);
                                float _e92 = cloud_slice(_e90, 0u);
                                float _e94 = cloud_slice(_e90, 20u);
                                float _e95 = puff_wave(_e92, _e94, cloud);
                                float _e97 = cloud_slice(_e90, 10u);
                                float _e99 = cloud_slice(_e73, 20u);
                                float _e100 = puff_wave(_e97, _e99, cloud);
                                wander = metal::float2(_e95, _e100);
                            }
                            metal::float2 _e106 = wander;
                            metal::float2 centre = (static_cast<metal::float2>(c) + metal::float2(0.5)) + ((place + (_e106 * 0.18)) * PUFF_JITTER);
                            metal::float2 x_3 = r - centre;
                            float u = 1.0 - (metal::dot(x_3, x_3) * inv_r2_);
                            if (u <= 0.0) {
                                continue;
                            }
                            float _e121 = cloud_slice(_e73, 20u);
                            float _e123 = shape;
                            float _e125 = amp;
                            float weight = metal::mix(_e121, _e121 * _e121, _e123) * _e125;
                            float lump = ((u * u) * u) * weight;
                            float _e134 = freq;
                            metal::float2 lump_slope = (((((-6.0 * u) * u) * weight) * _e134) * inv_r2_) * x_3;
                            float _e139 = out.depth;
                            out.depth = _e139 + lump;
                            float _e142 = out.broad_depth;
                            float _e143 = broad;
                            out.broad_depth = _e142 + (lump * _e143);
                            metal::float2 _e147 = out.slope;
                            float _e148 = broad;
                            out.slope = _e147 + (lump_slope * _e148);
                            int _e151 = octave;
                            if (_e151 == 0) {
                                float _e155 = out.body;
                                out.body = _e155 + lump;
                                metal::float2 _e158 = out.body_slope;
                                out.body_slope = _e158 + lump_slope;
                            }
                            int _e160 = octave;
                            if (_e160 == 2) {
                                float u2_ = u * u;
                                float reach_1 = u2_ * u2_;
                                metal::float2 _e165 = lit_acc;
                                float _e168 = freq;
                                lit_acc = _e165 + ((reach_1 * (centre - shift_1)) / metal::float2(_e168));
                                float _e173 = out.lit_weight;
                                out.lit_weight = _e173 + reach_1;
                            }
                        }
                    }
                }
            }
            float _e181 = freq;
            freq = _e181 * lacunarity;
            float _e183 = amp;
            amp = _e183 / lacunarity;
            float _e185 = broad;
            broad = _e185 * 0.25;
            float _e188 = shape;
            shape = _e188 * 0.35;
        }
    }
    float _e195 = out.lit_weight;
    if (_e195 > 0.0) {
        metal::float2 _e199 = lit_acc;
        float _e201 = out.lit_weight;
        out.lit_centre = _e199 / metal::float2(_e201);
    }
    Puffs _e204 = out;
    return _e204;
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
    return (1.3 * _e10) + (0.25 * material_1);
}

metal::float3 softened(
    metal::float3 colour
) {
    float m = metal::max(metal::max(colour.x, colour.y), colour.z);
    if (m <= 0.75) {
        return colour;
    }
    return colour * ((0.75 + (0.25 * (1.0 - metal::exp((0.75 - m) * 4.0)))) / m);
}

Backlight backlight(
    metal::float2 pt_1,
    float reach,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    Backlight out_1 = {};
    metal::float2 d0_ = metal::float2(0.2887, 0.0);
    metal::float2 d1_ = metal::float2(-0.3687, 0.3377);
    metal::float2 d2_ = metal::float2(0.0564, -0.643);
    metal::float2 d3_ = metal::float2(0.4647, 0.6061);
    metal::float2 d4_ = metal::float2(-0.8528, -0.1508);
    metal::float2 d5_ = metal::float2(0.8078, -0.5139);
    float _e22 = cloud_light(pt_1 + (d0_ * reach), close_light, wide_light, cloud_sampler, cloud);
    float _e25 = cloud_light(pt_1 + (d1_ * reach), close_light, wide_light, cloud_sampler, cloud);
    float _e28 = cloud_light(pt_1 + (d2_ * reach), close_light, wide_light, cloud_sampler, cloud);
    float _e31 = cloud_light(pt_1 + (d3_ * reach), close_light, wide_light, cloud_sampler, cloud);
    float _e34 = cloud_light(pt_1 + (d4_ * reach), close_light, wide_light, cloud_sampler, cloud);
    float _e37 = cloud_light(pt_1 + (d5_ * reach), close_light, wide_light, cloud_sampler, cloud);
    float ring = ((((_e22 + _e25) + _e28) + _e31) + _e34) + _e37;
    float _e43 = cloud_light(pt_1, close_light, wide_light, cloud_sampler, cloud);
    out_1.glow = (_e43 * 0.2) + (ring * 0.1333);
    metal::float2 pull = (((((metal::normalize(d0_) * _e22) + (metal::normalize(d1_) * _e25)) + (metal::normalize(d2_) * _e28)) + (metal::normalize(d3_) * _e31)) + (metal::normalize(d4_) * _e34)) + (metal::normalize(d5_) * _e37);
    float lean = metal::length(pull) / metal::max(ring, 0.0001);
    float followed = 0.6 * metal::smoothstep(0.02, 0.25, lean);
    out_1.toward = metal::normalize(metal::mix(metal::float2(0.55, -0.83), pull / metal::float2(metal::max(metal::length(pull), 0.00001)), followed));
    out_1.aimed = 0.45 + (0.55 * metal::smoothstep(0.0, 0.2, lean));
    Backlight _e96 = out_1;
    return _e96;
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
    metal::float2 pt_2 = (position_1 / metal::float2(_e21)) - _e26;
    float _e30 = cloud.cloud_scale;
    float units = 5.0 / _e30;
    metal::float2 _e35 = cloud.size;
    float _e42 = cloud.size.y;
    metal::float2 _e48 = cloud.drift;
    metal::float2 q = (((pt_2 - (_e35 * 0.5)) / metal::float2(_e42)) * units) + _e48;
    float _e52 = cloud.scale_size;
    float lacunarity_1 = metal::max(1.2, metal::sqrt(4.0 / _e52));
    float _e60 = cloud.scale_overlap;
    Puffs _e61 = puff_field(q, lacunarity_1, _e60, cloud);
    float _e64 = cloud.cloud_cover;
    float take = metal::mix(0.5, 11.0, _e64);
    float solid = _e61.broad_depth * 1.33;
    float alpha = 1.0 - metal::exp((-(take) * solid) * solid);
    if (alpha <= 0.002) {
        return base;
    }
    metal::float2 _e82 = cloud.drift;
    float _e89 = cloud.size.y;
    metal::float2 _e93 = cloud.size;
    metal::float2 centre_pt = (((_e61.lit_centre - _e82) / metal::float2(units)) * _e89) + (_e93 * 0.5);
    float quantized = 0.2 * metal::smoothstep(0.0, 0.1, _e61.lit_weight);
    float _e106 = cloud.size.y;
    float cloud_points = _e106 / units;
    float _e114 = cloud.size.y;
    Backlight _e118 = backlight(metal::mix(pt_2, centre_pt, quantized), metal::max(cloud_points * 2.0, _e114 * 0.2), close_light, wide_light, cloud_sampler, cloud);
    float _e123 = cloud.scale_glint;
    metal::float3 normal = metal::normalize(metal::float3(-(_e61.slope) * (1.8 * _e123), 1.0));
    metal::float3 sun = metal::normalize(metal::float3(_e118.toward * 0.7, 0.7));
    float _e137 = wrapped_light(metal::dot(normal, sun));
    float _e139 = wrapped_light(sun.z);
    float diffuse = _e137 / _e139;
    metal::float3 half_1 = metal::normalize(sun + metal::float3(0.0, 0.0, 1.0));
    float flat_glint = metal::pow(half_1.z, 6.0);
    float glint = (metal::max(metal::pow(metal::max(metal::dot(normal, half_1), 0.0), 6.0) - flat_glint, 0.0) / (1.0 - flat_glint)) * _e118.aimed;
    float through = metal::exp(-1.6 * _e61.depth);
    float rim = metal::clamp(metal::dot(-(_e61.body_slope), _e118.toward) * 0.7, 0.0, 1.0) * _e118.aimed;
    float shading = (diffuse * (0.45 + (0.9 * through))) * (0.8 + (0.5 * rim));
    float tint = metal::pow(metal::clamp(_e118.glow, 0.0, 1.0), 0.7);
    float _e198 = cloud.cloud_ambient;
    float level_5 = metal::clamp((tint * 0.85) + _e198, 0.0, 1.0);
    metal::float3 _e203 = palette_color(level_5, lut);
    float _e207 = cloud.scale_glint;
    metal::float3 body = (_e203 * shading) + metal::float3(((glint * _e207) * level_5) * 0.25);
    metal::float3 _e214 = softened(body);
    float _e217 = cloud.cloud_depth;
    return metal::mix(base, _e214, _e217 * alpha);
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

struct fs_cloud_gammaInput {
    float slab [[user(loc0), center_perspective]];
    float t [[user(loc1), center_perspective]];
};
struct fs_cloud_gammaOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_cloud_gammaOutput fs_cloud_gamma(
  fs_cloud_gammaInput varyings [[stage_in]]
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
    return fs_cloud_gammaOutput { _e1 };
}
