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
    float diffusion;
    float texture;
    float ppp;
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
    uint i = (slot * _e4) + bucket;
    uint _e11 = grid[metal::min(unsigned(i >> 2u), (_buffer_sizes.size1 - 0 - 4) / 4)];
    return (_e11 >> ((i & 3u) * 8u)) & 255u;
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
    float x = _e74 - 0.5;
    uint _e80 = locals.bins;
    uint b_2 = naga_f2u32(metal::clamp(metal::floor(x), 0.0, static_cast<float>(_e80) - 2.0));
    float f = metal::clamp(x - static_cast<float>(b_2), 0.0, 1.0);
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

metal::float3 linear_from_gamma_rgb(
    metal::float3 srgb
) {
    metal::bool3 cutoff = srgb < metal::float3(0.04045);
    metal::float3 lower = srgb / metal::float3(12.92);
    metal::float3 higher = metal::pow((srgb + metal::float3(0.055)) / metal::float3(1.055), metal::float3(2.4));
    return metal::select(higher, lower, cutoff);
}

metal::float2 baked_density(
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
    return _e17.xy;
}

float diffused_level(
    float core,
    metal::float2 material,
    constant Cloud& cloud
) {
    float retain = metal::smoothstep(0.45, 0.95, core);
    float _e8 = cloud.diffusion;
    float _e12 = cloud.diffusion;
    float level_2 = metal::mix(core, material.x, _e8) + ((_e12 * retain) * metal::max(core - material.x, 0.0));
    float _e22 = cloud.texture;
    float _e25 = cloud.diffusion;
    float texture = (_e22 * _e25) * (1.0 - metal::smoothstep(0.7, 1.0, level_2));
    return level_2 * metal::mix(1.0, material.y, texture);
}

metal::float4 density_color(
    float level,
    metal::texture2d<float, metal::access::sample> lut
) {
    uint levels = metal::uint2(lut.get_width(), lut.get_height()).x;
    float x_1 = (metal::clamp(level, 0.0, 1.0) * static_cast<float>(levels)) - 0.5;
    uint i_1 = naga_f2u32(metal::clamp(metal::floor(x_1), 0.0, static_cast<float>(levels - 1u)));
    uint clamped_lod_e22 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e22 = lut.read(metal::min(metal::uint2(metal::uint2(i_1, 0u)), metal::uint2(lut.get_width(clamped_lod_e22), lut.get_height(clamped_lod_e22)) - 1), clamped_lod_e22);
    metal::float3 a = _e22.xyz;
    if (x_1 < 0.0) {
        return metal::float4((a * (x_1 + 0.5)) * 2.0, 1.0);
    }
    uint clamped_lod_e42 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e42 = lut.read(metal::min(metal::uint2(metal::uint2(metal::min(i_1 + 1u, levels - 1u), 0u)), metal::uint2(lut.get_width(clamped_lod_e42), lut.get_height(clamped_lod_e42)) - 1), clamped_lod_e42);
    metal::float3 b_3 = _e42.xyz;
    return metal::float4(metal::mix(a, b_3, metal::fract(x_1)), 1.0);
}

metal::float4 cloud_color(
    VertexOut in_2,
    constant Locals& locals,
    device type_3 const& grid,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    constant _mslBufferSizes& _buffer_sizes
) {
    float _e1 = heatmap_level(in_2, locals, grid, _buffer_sizes);
    metal::float2 _e4 = baked_density(in_2.position.xy, close_light, cloud_sampler, cloud);
    float _e5 = diffused_level(_e1, _e4, cloud);
    metal::float4 _e6 = density_color(_e5, lut);
    return _e6;
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
, metal::texture2d<float, metal::access::sample> close_light [[texture(1)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(2)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    const VertexOut in = { position_1, varyings.slab, varyings.t };
    metal::float4 _e1 = cloud_color(in, locals, grid, lut, close_light, cloud_sampler, cloud, _buffer_sizes);
    metal::float3 _e3 = linear_from_gamma_rgb(_e1.xyz);
    return fs_cloud_linearOutput { metal::float4(_e3, _e1.w) };
}
