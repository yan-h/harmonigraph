// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint size1;
    uint size3;
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
    uint detail;
    float stroke_sigma;
    float prominence_steps;
    uint peak_stride;
    float gather_sigma_slabs;
    uint peak_header;
    uint peak_bands;
    float peak_band;
    uint _pad0_;
    uint _pad1_;
    uint _pad2_;
};
typedef uint type_3[1];
typedef metal::float4 type_6[1];
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
    float diffusion;
    float ppp;
    float cloud;
    float _pad0_;
    float _pad1_;
    float _pad2_;
};

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

float bucket_level(
    uint slot_1,
    uint b,
    constant Locals& locals,
    device type_3 const& grid,
    constant _mslBufferSizes& _buffer_sizes
) {
    float _e4 = locals.spectrum_min_midi;
    float _e10 = locals.bins_per_semitone;
    float midi_1 = _e4 + ((static_cast<float>(b) + 0.5) / _e10);
    uint _e13 = stored(slot_1, b, locals, grid, _buffer_sizes);
    float v = static_cast<float>(_e13);
    float _e17 = locals.level0_;
    float _e20 = locals.level_per_step;
    float _e25 = locals.level_per_midi;
    float level_1 = (_e17 + (_e20 * v)) + (_e25 * midi_1);
    return metal::clamp(level_1, 0.0, 1.0);
}

uint naga_f2u32(float value) {
    return static_cast<uint>(metal::clamp(value, 0.0, 4294967000.0));
}

float read_level(
    uint slot_2,
    float t_1,
    constant Locals& locals,
    device type_3 const& grid,
    constant _mslBufferSizes& _buffer_sizes
) {
    float sum = 0.0;
    float total = 0.0;
    uint b_1 = {};
    uint _e4 = locals.rows;
    float half_ = 0.5 / static_cast<float>(_e4);
    float _e9 = bucket_x(t_1 - half_, locals);
    float _e11 = bucket_x(t_1 + half_, locals);
    uint _e14 = locals.bins;
    float top = static_cast<float>(_e14) - 1.0;
    uint idx = naga_f2u32(metal::clamp(metal::floor(_e9), 0.0, top));
    uint last = naga_f2u32(metal::clamp(metal::floor(_e11), 0.0, top));
    if (last > idx) {
        uint _e29 = locals.bins;
        float lo = metal::clamp(_e9, 0.0, static_cast<float>(_e29));
        uint _e35 = locals.bins;
        float hi = metal::clamp(_e11, 0.0, static_cast<float>(_e35));
        b_1 = idx;
        uint2 loop_bound = uint2(4294967295u);
        bool loop_init = true;
        while(true) {
            if (metal::all(loop_bound == uint2(0u))) { break; }
            loop_bound -= uint2(loop_bound.y == 0u, 1u);
            if (!loop_init) {
                uint _e64 = b_1;
                b_1 = _e64 + 1u;
            }
            loop_init = false;
            uint _e44 = b_1;
            if (_e44 <= last) {
            } else {
                break;
            }
            {
                uint _e46 = b_1;
                uint _e51 = b_1;
                float w = metal::max(metal::min(hi, static_cast<float>(_e46) + 1.0) - metal::max(lo, static_cast<float>(_e51)), 0.0);
                float _e57 = sum;
                uint _e58 = b_1;
                float _e59 = bucket_level(slot_2, _e58, locals, grid, _buffer_sizes);
                sum = _e57 + (w * _e59);
                float _e62 = total;
                total = _e62 + w;
            }
        }
        float _e67 = total;
        if (_e67 <= 0.0) {
            float _e70 = bucket_level(slot_2, idx, locals, grid, _buffer_sizes);
            return _e70;
        }
        float _e71 = sum;
        float _e72 = total;
        return _e71 / _e72;
    }
    float _e74 = bucket_x(t_1, locals);
    float x_1 = _e74 - 0.5;
    uint _e80 = locals.bins;
    uint b_2 = naga_f2u32(metal::clamp(metal::floor(x_1), 0.0, static_cast<float>(_e80) - 2.0));
    float f = metal::clamp(x_1 - static_cast<float>(b_2), 0.0, 1.0);
    float _e92 = bucket_level(slot_2, b_2, locals, grid, _buffer_sizes);
    float _e95 = bucket_level(slot_2, b_2 + 1u, locals, grid, _buffer_sizes);
    return metal::mix(_e92, _e95, f);
}

uint naga_mod(uint lhs, uint rhs) {
    return lhs % metal::select(rhs, 1u, rhs == 0u);
}

float heatmap_level(
    VertexOut in_1,
    constant Locals& locals,
    device type_3 const& grid,
    constant _mslBufferSizes& _buffer_sizes
) {
    uint _e3 = locals.run_slabs;
    float n = static_cast<float>(_e3);
    float jx = metal::clamp(metal::floor(in_1.slab - 0.5), 0.0, n - 1.0);
    uint j0_ = naga_f2u32(jx);
    uint _e18 = locals.run_slabs;
    uint j1_ = metal::min(j0_ + 1u, _e18 - 1u);
    float fx = metal::clamp((in_1.slab - 0.5) - jx, 0.0, 1.0);
    uint _e31 = locals.first_slot;
    uint _e35 = locals.capacity;
    uint s0_ = naga_mod(_e31 + j0_, _e35);
    uint _e39 = locals.first_slot;
    uint _e43 = locals.capacity;
    uint s1_ = naga_mod(_e39 + j1_, _e43);
    float _e46 = read_level(s0_, in_1.t, locals, grid, _buffer_sizes);
    float _e48 = read_level(s1_, in_1.t, locals, grid, _buffer_sizes);
    return metal::mix(_e46, _e48, fx);
}

float peak_level(
    metal::float4 peak,
    constant Locals& locals
) {
    float _e3 = locals.spectrum_min_midi;
    float _e7 = locals.bins_per_semitone;
    float midi_2 = _e3 + (peak.x / _e7);
    float _e12 = locals.level0_;
    float _e15 = locals.level_per_step;
    float _e21 = locals.level_per_midi;
    float level_2 = (_e12 + (_e15 * peak.y)) + (_e21 * midi_2);
    return metal::clamp(level_2, 0.0, 1.0);
}

uint band_start(
    uint base,
    uint k,
    device type_6 const& peaks,
    constant _mslBufferSizes& _buffer_sizes
) {
    float _e10 = peaks[metal::min(unsigned(base + (k >> 2u)), (_buffer_sizes.size3 - 0 - 16) / 16)][metal::min(unsigned(k & 3u), 3u)];
    return naga_f2u32(_e10);
}

float slab_stroke(
    uint slot_3,
    float x,
    float reach,
    float falloff,
    constant Locals& locals,
    device type_6 const& peaks,
    constant _mslBufferSizes& _buffer_sizes
) {
    uint i = {};
    float best = 0.0;
    bool local = {};
    uint _e6 = locals.peak_stride;
    uint base_1 = slot_3 * _e6;
    uint _e10 = locals.peak_bands;
    uint _e11 = band_start(base_1, _e10, peaks, _buffer_sizes);
    float hi_1 = x + reach;
    float _e16 = locals.peak_band;
    uint _e21 = locals.peak_bands;
    uint band = naga_f2u32(metal::clamp(metal::floor((x - reach) / _e16), 0.0, static_cast<float>(_e21 - 1u)));
    uint _e30 = locals.peak_header;
    uint first = base_1 + _e30;
    uint _e32 = band_start(base_1, band, peaks, _buffer_sizes);
    i = _e32;
    uint2 loop_bound_1 = uint2(4294967295u);
    while(true) {
        if (metal::all(loop_bound_1 == uint2(0u))) { break; }
        loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
        uint _e36 = i;
        if (_e36 < _e11) {
        } else {
            break;
        }
        {
            uint _e39 = i;
            metal::float4 peak_1 = peaks[metal::min(unsigned(first + _e39), (_buffer_sizes.size3 - 0 - 16) / 16)];
            if (peak_1.x > hi_1) {
                break;
            }
            float d = x - peak_1.x;
            if (metal::abs(d) <= reach) {
                float _e56 = locals.prominence_steps;
                local = (peak_1.y - peak_1.z) >= _e56;
            } else {
                local = false;
            }
            bool _e59 = local;
            if (_e59) {
                float _e60 = best;
                float _e61 = peak_level(peak_1, locals);
                best = metal::max(_e60, _e61 * metal::exp((-(d) * d) * falloff));
            }
            uint _e68 = i;
            i = _e68 + 1u;
        }
    }
    float _e71 = best;
    return _e71;
}

int naga_f2i32(float value) {
    return static_cast<int>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

int naga_neg(int val) {
    return as_type<int>(-as_type<uint>(val));
}

float stroke_level(
    VertexOut in_2,
    constant Locals& locals,
    device type_6 const& peaks,
    constant _mslBufferSizes& _buffer_sizes
) {
    float sum_1 = 0.0;
    float total_1 = 0.0;
    int k_1 = {};
    bool local_1 = {};
    uint _e3 = locals.run_slabs;
    int n_1 = static_cast<int>(_e3);
    int jc = naga_f2i32(metal::clamp(metal::floor(in_2.slab), 0.0, static_cast<float>(n_1) - 1.0));
    float _e14 = bucket_x(in_2.t, locals);
    float _e17 = locals.stroke_sigma;
    float reach_1 = 3.0 * _e17;
    float _e22 = locals.stroke_sigma;
    float _e27 = locals.stroke_sigma;
    float falloff_1 = 1.0 / ((2.0 * _e22) * _e27);
    float _e33 = locals.gather_sigma_slabs;
    float sigma = metal::max(_e33, 0.001);
    int radius = naga_f2i32(metal::ceil(2.0 * sigma));
    float time_falloff = 1.0 / ((2.0 * sigma) * sigma);
    k_1 = naga_neg(radius);
    uint2 loop_bound_2 = uint2(4294967295u);
    bool loop_init_1 = true;
    while(true) {
        if (metal::all(loop_bound_2 == uint2(0u))) { break; }
        loop_bound_2 -= uint2(loop_bound_2.y == 0u, 1u);
        if (!loop_init_1) {
            int _e85 = k_1;
            k_1 = as_type<int>(as_type<uint>(_e85) + as_type<uint>(1));
        }
        loop_init_1 = false;
        int _e51 = k_1;
        if (_e51 <= radius) {
        } else {
            break;
        }
        {
            int _e53 = k_1;
            int j = as_type<int>(as_type<uint>(jc) + as_type<uint>(_e53));
            if (!((j < 0))) {
                local_1 = j >= n_1;
            } else {
                local_1 = true;
            }
            bool _e62 = local_1;
            if (_e62) {
                continue;
            }
            int _e63 = k_1;
            int _e64 = k_1;
            float weight = metal::exp(-(static_cast<float>(as_type<int>(as_type<uint>(_e63) * as_type<uint>(_e64)))) * time_falloff);
            uint _e72 = locals.first_slot;
            uint _e77 = locals.capacity;
            uint slot_4 = naga_mod(_e72 + static_cast<uint>(j), _e77);
            float _e79 = sum_1;
            float _e80 = slab_stroke(slot_4, _e14, reach_1, falloff_1, locals, peaks, _buffer_sizes);
            sum_1 = _e79 + (weight * _e80);
            float _e83 = total_1;
            total_1 = _e83 + weight;
        }
    }
    float _e88 = sum_1;
    float _e89 = total_1;
    float _e91 = total_1;
    return (_e91 > 0.0) ? (_e88 / _e89) : 0.0;
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

float diffused_level(
    float core,
    float material,
    constant Cloud& cloud
) {
    float _e4 = cloud.diffusion;
    float _e9 = cloud.diffusion;
    float raw = (1.0 - _e4) * (1.0 - _e9);
    return metal::mix(material, core, raw);
}

metal::float4 density_color(
    float level,
    metal::texture2d<float, metal::access::sample> lut
) {
    uint levels = metal::uint2(lut.get_width(), lut.get_height()).x;
    float x_2 = (metal::clamp(level, 0.0, 1.0) * static_cast<float>(levels)) - 0.5;
    uint i_2 = naga_f2u32(metal::clamp(metal::floor(x_2), 0.0, static_cast<float>(levels - 1u)));
    uint clamped_lod_e22 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e22 = lut.read(metal::min(metal::uint2(metal::uint2(i_2, 0u)), metal::uint2(lut.get_width(clamped_lod_e22), lut.get_height(clamped_lod_e22)) - 1), clamped_lod_e22);
    metal::float3 a = _e22.xyz;
    if (x_2 < 0.0) {
        return metal::float4((a * (x_2 + 0.5)) * 2.0, 1.0);
    }
    uint clamped_lod_e42 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e42 = lut.read(metal::min(metal::uint2(metal::uint2(metal::min(i_2 + 1u, levels - 1u), 0u)), metal::uint2(lut.get_width(clamped_lod_e42), lut.get_height(clamped_lod_e42)) - 1), clamped_lod_e42);
    metal::float3 b_3 = _e42.xyz;
    return metal::float4(metal::mix(a, b_3, metal::fract(x_2)), 1.0);
}

metal::float4 cloud_color(
    VertexOut in_3,
    constant Locals& locals,
    device type_3 const& grid,
    metal::texture2d<float, metal::access::sample> lut,
    device type_6 const& peaks,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    constant _mslBufferSizes& _buffer_sizes
) {
    float _e3 = baked_density(in_3.position.xy, close_light, cloud_sampler, cloud);
    uint _e6 = locals.detail;
    if (_e6 == 1u) {
        float _e9 = stroke_level(in_3, locals, peaks, _buffer_sizes);
        float _e12 = cloud.cloud;
        metal::float4 _e15 = density_color(metal::max(_e9, _e12 * _e3), lut);
        return _e15;
    }
    float _e16 = heatmap_level(in_3, locals, grid, _buffer_sizes);
    float _e17 = diffused_level(_e16, _e3, cloud);
    metal::float4 _e18 = density_color(_e17, lut);
    return _e18;
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
, metal::float4 position_1 [[position]]
, constant Locals& locals [[buffer(0)]]
, device type_3 const& grid [[buffer(1)]]
, metal::texture2d<float, metal::access::sample> lut [[texture(0)]]
, device type_6 const& peaks [[buffer(2)]]
, metal::texture2d<float, metal::access::sample> close_light [[texture(1)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(3)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(4)]]
) {
    const VertexOut in = { position_1, varyings.slab, varyings.t };
    metal::float4 _e1 = cloud_color(in, locals, grid, lut, peaks, close_light, cloud_sampler, cloud, _buffer_sizes);
    metal::float3 _e3 = linear_from_gamma_rgb(_e1.xyz);
    return fs_cloud_linearOutput { metal::float4(_e3, _e1.w) };
}
