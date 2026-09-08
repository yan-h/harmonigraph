// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct CompositeParams {
    float darkest_pitch;
    float brightest_pitch;
    float render_scale;
    float bloom_strength;
};
struct BlitOut {
    metal::float4 pos;
    metal::float2 uv;
    char _pad2[8];
};
constant float BLOOM_THRESHOLD = 0.35;
constant float BLOOM_KNEE = 0.25;
constant float BLUR_W0_ = 0.227027;
constant metal::float4 BLUR_W = metal::float4(0.1945946, 0.1216216, 0.054054, 0.016216);

float interleaved_gradient_noise(
    metal::float2 pixel
) {
    float f = (0.06711056 * pixel.x) + (0.00583715 * pixel.y);
    return metal::fract(52.982918 * metal::fract(f));
}

metal::float3 dither_to_unorm8_(
    metal::float3 rgb,
    metal::float2 pixel_1
) {
    float _e2 = interleaved_gradient_noise(pixel_1);
    float noise = (_e2 - 0.5) * 0.85;
    return rgb + metal::float3(noise / 255.0);
}

struct fs_compositeInput {
    metal::float2 uv [[user(loc0), center_perspective]];
};
struct fs_compositeOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_compositeOutput fs_composite(
  fs_compositeInput varyings [[stage_in]]
, metal::float4 pos [[position]]
, metal::texture2d<float, metal::access::sample> scene_tex [[texture(0)]]
, metal::sampler scene_samp [[sampler(0)]]
, metal::texture2d<float, metal::access::sample> bloom_tex [[texture(1)]]
, constant CompositeParams& bu [[buffer(0)]]
) {
    const BlitOut in = { pos, varyings.uv };
    metal::float4 scene = scene_tex.sample(scene_samp, in.uv);
    metal::float4 bloom = bloom_tex.sample(scene_samp, in.uv);
    float _e13 = bu.bloom_strength;
    metal::float3 rgb_1 = scene.xyz + (bloom.xyz * _e13);
    bool has_light = metal::any(rgb_1 > metal::float3(0.0));
    metal::float3 _e22 = dither_to_unorm8_(rgb_1, in.pos.xy);
    metal::float3 dithered = has_light ? _e22 : rgb_1;
    return fs_compositeOutput { metal::float4(dithered, scene.w) };
}
