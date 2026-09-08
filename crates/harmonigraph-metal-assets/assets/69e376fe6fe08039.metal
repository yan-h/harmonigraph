// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct VertexOutput {
    metal::float2 tex_coord;
    char _pad1[8];
    metal::float4 color;
    metal::float4 position;
};
struct Locals {
    metal::float2 screen_size;
    uint dithering;
    uint predictable_texture_filtering;
};

float interleaved_gradient_noise(
    metal::float2 n
) {
    float f = (0.06711056 * n.x) + (0.00583715 * n.y);
    return metal::fract(52.982918 * metal::fract(f));
}

metal::float3 dither_interleaved(
    metal::float3 rgb,
    float levels,
    metal::float4 frag_coord
) {
    float noise = {};
    float _e4 = interleaved_gradient_noise(frag_coord.xy);
    noise = _e4;
    float _e6 = noise;
    noise = (_e6 - 0.5) * 0.95;
    float _e11 = noise;
    return rgb + metal::float3(_e11 / (levels - 1.0));
}

metal::float3 linear_from_gamma_rgb(
    metal::float3 srgb
) {
    metal::bool3 cutoff = srgb < metal::float3(0.04045);
    metal::float3 lower = srgb / metal::float3(12.92);
    metal::float3 higher = metal::pow((srgb + metal::float3(0.055)) / metal::float3(1.055), metal::float3(2.4));
    return metal::select(higher, lower, cutoff);
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

metal::float4 sample_texture(
    VertexOutput in_1,
    constant Locals& r_locals,
    metal::texture2d<float, metal::access::sample> r_tex_color,
    metal::sampler r_tex_sampler
) {
    uint _e3 = r_locals.predictable_texture_filtering;
    if (_e3 == 0u) {
        metal::float4 _e9 = r_tex_color.sample(r_tex_sampler, in_1.tex_coord);
        return _e9;
    } else {
        metal::int2 texture_size = static_cast<metal::int2>(metal::uint2(r_tex_color.get_width(0), r_tex_color.get_height(0)));
        metal::float2 texture_size_f = static_cast<metal::float2>(texture_size);
        metal::float2 pixel_coord = (in_1.tex_coord * texture_size_f) - metal::float2(0.5);
        metal::float2 pixel_fract = metal::fract(pixel_coord);
        metal::int2 pixel_floor = naga_f2i32(metal::floor(pixel_coord));
        metal::int2 max_coord = as_type<metal::int2>(as_type<metal::uint2>(texture_size) - as_type<metal::uint2>(metal::int2(1, 1)));
        metal::int2 p00_ = metal::clamp(as_type<metal::int2>(as_type<metal::uint2>(pixel_floor) + as_type<metal::uint2>(metal::int2(0, 0))), metal::int2(0, 0), max_coord);
        metal::int2 p10_ = metal::clamp(as_type<metal::int2>(as_type<metal::uint2>(pixel_floor) + as_type<metal::uint2>(metal::int2(1, 0))), metal::int2(0, 0), max_coord);
        metal::int2 p01_ = metal::clamp(as_type<metal::int2>(as_type<metal::uint2>(pixel_floor) + as_type<metal::uint2>(metal::int2(0, 1))), metal::int2(0, 0), max_coord);
        metal::int2 p11_ = metal::clamp(as_type<metal::int2>(as_type<metal::uint2>(pixel_floor) + as_type<metal::uint2>(metal::int2(1, 1))), metal::int2(0, 0), max_coord);
        uint clamped_lod_e61 = metal::min(uint(0), r_tex_color.get_num_mip_levels() - 1);
        metal::float4 tl = r_tex_color.read(metal::min(metal::uint2(p00_), metal::uint2(r_tex_color.get_width(clamped_lod_e61), r_tex_color.get_height(clamped_lod_e61)) - 1), clamped_lod_e61);
        uint clamped_lod_e64 = metal::min(uint(0), r_tex_color.get_num_mip_levels() - 1);
        metal::float4 tr = r_tex_color.read(metal::min(metal::uint2(p10_), metal::uint2(r_tex_color.get_width(clamped_lod_e64), r_tex_color.get_height(clamped_lod_e64)) - 1), clamped_lod_e64);
        uint clamped_lod_e67 = metal::min(uint(0), r_tex_color.get_num_mip_levels() - 1);
        metal::float4 bl = r_tex_color.read(metal::min(metal::uint2(p01_), metal::uint2(r_tex_color.get_width(clamped_lod_e67), r_tex_color.get_height(clamped_lod_e67)) - 1), clamped_lod_e67);
        uint clamped_lod_e70 = metal::min(uint(0), r_tex_color.get_num_mip_levels() - 1);
        metal::float4 br = r_tex_color.read(metal::min(metal::uint2(p11_), metal::uint2(r_tex_color.get_width(clamped_lod_e70), r_tex_color.get_height(clamped_lod_e70)) - 1), clamped_lod_e70);
        metal::float4 top = metal::mix(tl, tr, pixel_fract.x);
        metal::float4 bottom = metal::mix(bl, br, pixel_fract.x);
        return metal::mix(top, bottom, pixel_fract.y);
    }
}

struct fs_main_linear_framebufferInput {
    metal::float2 tex_coord [[user(loc0), center_perspective]];
    metal::float4 color [[user(loc1), center_perspective]];
};
struct fs_main_linear_framebufferOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_main_linear_framebufferOutput fs_main_linear_framebuffer(
  fs_main_linear_framebufferInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Locals& r_locals [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> r_tex_color [[texture(0)]]
, metal::sampler r_tex_sampler [[sampler(0)]]
) {
    const VertexOutput in = { varyings.tex_coord, {}, varyings.color, position };
    metal::float4 out_color_gamma = {};
    metal::float4 _e1 = sample_texture(in, r_locals, r_tex_color, r_tex_sampler);
    out_color_gamma = in.color * _e1;
    uint _e7 = r_locals.dithering;
    if (_e7 == 1u) {
        metal::float4 _e10 = out_color_gamma;
        metal::float3 _e14 = dither_interleaved(_e10.xyz, 256.0, in.position);
        float _e16 = out_color_gamma.w;
        out_color_gamma = metal::float4(_e14, _e16);
    }
    metal::float4 _e18 = out_color_gamma;
    metal::float3 _e20 = linear_from_gamma_rgb(_e18.xyz);
    float _e22 = out_color_gamma.w;
    return fs_main_linear_framebufferOutput { metal::float4(_e20, _e22) };
}
